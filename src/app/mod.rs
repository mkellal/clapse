use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{self, Line, Text};
use ratatui::widgets::{Tabs, Widget};
use std::rc::Rc;

pub mod span;
pub mod tabs;
pub mod view;

use crate::widgets::help::HelpPopup;
use crate::widgets::search::SearchState;
use crate::app::span::Span;
use crate::app::tabs::flamegraph::FlameGraphTab;
use crate::app::tabs::sources::SourcesTab;
use crate::app::tabs::templates::TemplatesTab;
use crate::app::view::{LoadProgress, load_spans_with_progress};
use crate::cli;
use crate::widgets::start_screen::StartScreenWidget;

/// RAII guard that enables mouse capture on creation and disables it on drop.
struct MouseCaptureGuard;

impl MouseCaptureGuard {
    fn enable() -> std::io::Result<Self> {
        execute!(std::io::stdout(), EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for MouseCaptureGuard {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
}

/// Holds the state while spans are being loaded in a background thread.
struct LoadingState {
    progress_rx: std::sync::mpsc::Receiver<LoadProgress>,
    current: LoadProgress,
    thread: Option<std::thread::JoinHandle<Vec<Span>>>,
}

pub struct App {
    current_tab_index: usize,
    tabs: Vec<Box<dyn tabs::Tab>>,
    tabs_area: Rect,
    show_help: bool,
    search: SearchState,
    /// Present while spans are being loaded from disk.
    loading: Option<LoadingState>,
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Render loading screen if spans are still being loaded.
        if let Some(ref loading) = self.loading {
            let total = loading.current.total_bytes.max(1) as f64;
            let progress = loading.current.bytes_processed as f64 / total;
            let msg = if loading.current.total_files > 0 {
                format!(
                    "Parsing files… ({}/{})",
                    loading.current.files_processed,
                    loading.current.total_files
                )
            } else {
                String::from("Discovering trace files…")
            };
            StartScreenWidget { progress, message: msg }.render(area, buf);
            return;
        }

        let main_layout = if self.search.visible {
            Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(3),
            ])
            .split(area)
        } else {
            std::rc::Rc::from([area])
        };

        let content_area = main_layout[0];

        let layout = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]);
        let [top, main] = content_area.layout(&layout);

        let top_layout =
            Layout::horizontal([Constraint::Max(8), Constraint::Fill(2), Constraint::Fill(1)]);
        let [title_area, tabs_area, help_area] = top.layout(&top_layout);
        let tabs = Tabs::new(
            self.tabs
                .iter()
                .map(|t| " ".to_string() + t.get_label() + " ")
                .collect::<Vec<_>>(),
        )
        .style(Color::White)
        .highlight_style(Style::default().black().on_light_green().bold())
        .select(self.current_tab_index)
        .divider("|")
        .padding(" ", " ");

        let title = Text::from("Clapse").bold();
        let help = Text::from(Line::from(vec![
            text::Span::styled("<", Color::DarkGray),
            text::Span::styled("?", Color::Red),
            text::Span::styled("> ", Color::DarkGray),
            text::Span::raw("Help ℹ️"),
        ]))
        .right_aligned();

        title.render(title_area, buf);
        tabs.render(tabs_area, buf);
        help.render(help_area, buf);

        self.tabs_area = tabs_area;

        let show_help = self.show_help;

        // Render tab content — use disjoint borrow for search afterward.
        self.tabs[self.current_tab_index].render(main, buf);

        if self.search.visible {
            let tab: &dyn tabs::Tab = &*self.tabs[self.current_tab_index];
            self.search.render(main_layout[1], buf, tab);
        }

        if show_help {
            let help_popup = HelpPopup::new(self.tabs[self.current_tab_index].get_help());
            help_popup.render(area, buf);
        }
    }
}

impl Default for App {
    fn default() -> Self {
        let cli = cli::Cli::parse();
        let build_dir = cli.build_dir.clone();

        let (progress_tx, progress_rx) = std::sync::mpsc::channel();

        let thread = std::thread::spawn(move || {
            load_spans_with_progress(&build_dir, progress_tx)
        });

        Self {
            current_tab_index: 0,
            tabs: Vec::new(),
            tabs_area: Rect::default(),
            show_help: false,
            search: SearchState::default(),
            loading: Some(LoadingState {
                progress_rx,
                current: LoadProgress {
                    bytes_processed: 0,
                    total_bytes: 1,
                    files_processed: 0,
                    total_files: 0,
                },
                thread: Some(thread),
            }),
        }
    }
}

impl App {
    pub fn run(mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let _mouse_guard = MouseCaptureGuard::enable()?;
        self.event_loop(terminal)
    }

    fn get_current_tab(&mut self) -> &mut dyn tabs::Tab {
        self.tabs[self.current_tab_index].as_mut()
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            // ── poll loading progress ──────────────────────────────────
            if let Some(ref mut loading) = self.loading {
                // Drain all pending progress messages.
                while let Ok(progress) = loading.progress_rx.try_recv() {
                    loading.current = progress;
                }

                // Check whether the background thread has finished.
                if let Some(handle) = loading.thread.take() {
                    if handle.is_finished() {
                        let raw_spans = handle.join().unwrap();
                        let raw_spans: Rc<[Span]> = Rc::from(raw_spans);
                        let tabs: Vec<Box<dyn tabs::Tab>> = vec![
                            Box::new(FlameGraphTab::new(raw_spans.clone())),
                            Box::new(SourcesTab::new(raw_spans.clone())),
                            Box::new(TemplatesTab::new(raw_spans.clone())),
                        ];
                        self.tabs = tabs;
                        self.loading = None;
                    } else {
                        // Put the handle back — thread still running.
                        loading.thread = Some(handle);
                    }
                }
            }

            // ── render ─────────────────────────────────────────────────
            let app = &mut *self;
            terminal.draw(|frame| frame.render_widget(&mut *app, frame.area()))?;

            // ── input ──────────────────────────────────────────────────
            // During loading, poll with a timeout so the progress bar
            // redraws even when the user isn't pressing keys.
            let event = if self.loading.is_some() {
                if crossterm::event::poll(std::time::Duration::from_millis(100))? {
                    Some(crossterm::event::read()?)
                } else {
                    None
                }
            } else {
                Some(crossterm::event::read()?)
            };

            match event {
                Some(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    // During loading, only quit keys are handled.
                    if self.loading.is_some() {
                        match key.code {
                            KeyCode::Char('q' | 'Q') => return Ok(()),
                            KeyCode::Char('c' | 'C')
                                if key
                                    .modifiers
                                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
                            {
                                return Ok(())
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if self.handle_key_event(key) {
                        return Ok(());
                    }
                }
                Some(Event::Mouse(mouse)) => {
                    if self.loading.is_some() {
                        continue;
                    }
                    self.handle_mouse_event(mouse);
                }
                None => {} // poll timeout — just re-render
                _ => {}
            }
        }
    }

    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        let ctrl = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(crossterm::event::KeyModifiers::ALT);

        // Delegate to search first — use disjoint field borrows.
        {
            if self.search.handle_key(key, &mut *self.tabs[self.current_tab_index]) {
                return false;
            }
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return true,
            KeyCode::Char('c') | KeyCode::Char('C') if ctrl => return true,
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                return false;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.search.open(&mut *self.tabs[self.current_tab_index]);
                return false;
            }
            KeyCode::Char(c @ ('1' | '2' | '3')) if alt => {
                let idx = (c as u8 - b'1') as usize;
                if idx < self.tabs.len() {
                    self.current_tab_index = idx;
                }
                return false;
            }
            KeyCode::Char('t') | KeyCode::Char('T') if alt => {
                self.current_tab_index = (self.current_tab_index + 1) % self.tabs.len();
                return false;
            }
            _ => {
                if self.show_help {
                    self.show_help = false;
                    return false;
                }
                let current_tab = self.get_current_tab();
                return current_tab.handle_key_event(key);
            }
        }
    }

    fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent) {
        if mouse.kind == crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)
            && mouse.row == self.tabs_area.y
            && mouse.column >= self.tabs_area.left()
            && mouse.column < self.tabs_area.right()
        {
            let mut current_x = self.tabs_area.x + 1; // +1 for padding(" ", " ")
            for (i, tab) in self.tabs.iter().enumerate() {
                let label_width = tab.get_label().len() as u16 + 2; // " " + label + " "
                if mouse.column >= current_x && mouse.column < current_x + label_width {
                    self.current_tab_index = i;
                    return;
                }
                current_x += label_width + 1; // +1 for divider "|"
            }
        }

        let current_tab = self.get_current_tab();
        current_tab.handle_mouse_event(mouse);
    }
}
