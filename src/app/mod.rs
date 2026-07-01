use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::event::{Event, KeyCode, KeyEventKind};
use crossterm::execute;
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{self, Line, Text};
use ratatui::widgets::{Tabs, Widget};
use std::rc::Rc;

pub mod help;
pub mod search;
pub mod span;
pub mod tabs;
pub mod view;

use crate::app::help::HelpPopup;
use crate::app::search::SearchState;
use crate::app::span::Span;
use crate::app::tabs::flamegraph::FlameGraphTab;
use crate::app::tabs::sources::SourcesTab;
use crate::app::tabs::templates::TemplatesTab;
use crate::app::view::load_spans;
use crate::cli;

enum ZoomDirection {
    In,
    Out,
}

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

pub struct App {
    // raw_spans: Rc<[Span]>,
    current_tab_index: usize,
    tabs: Vec<Box<dyn tabs::Tab>>,
    tabs_area: Rect,
    show_help: bool,
    search: SearchState,
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
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
            text::Span::styled("h", Color::Red),
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
        let spans: Vec<Span> = load_spans(&cli.build_dir);

        let raw_spans: Rc<[Span]> = Rc::from(spans);

        let tabs = vec![
            Box::new(FlameGraphTab::new(raw_spans.clone())) as Box<dyn tabs::Tab>,
            Box::new(SourcesTab::new(raw_spans.clone())) as Box<dyn tabs::Tab>,
            Box::new(TemplatesTab::new(raw_spans.clone())) as Box<dyn tabs::Tab>,
        ];

        Self {
            // raw_spans,
            current_tab_index: 0,
            tabs,
            tabs_area: Rect::default(),
            show_help: false,
            search: SearchState::default(),
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
            let app = &mut *self;
            terminal.draw(|frame| frame.render_widget(&mut *app, frame.area()))?;
            match crossterm::event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if self.handle_key_event(key) {
                        break Ok(());
                    }
                }
                Event::Mouse(mouse) => self.handle_mouse_event(mouse),
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
            KeyCode::Char('h') | KeyCode::Char('H') => {
                self.show_help = !self.show_help;
                return false;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.search.open(&mut *self.tabs[self.current_tab_index]);
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
