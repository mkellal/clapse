use glob::glob;
use std::path::PathBuf;

pub fn get_trace_files(build_dir: &PathBuf) -> Vec<PathBuf> {
    let pattern = format!("{}/**/*.*.json", build_dir.display());
    let paths: Vec<PathBuf> = match glob(&pattern) {
        Ok(entries) => entries
            .filter_map(|entry| match entry {
                Ok(path) => {
                    let object_path = path.with_extension("o");
                    if path.is_file() && object_path.exists() {
                        Some(path)
                    } else {
                        None
                    }
                }
                Err(e) => {
                    eprintln!("Error reading glob entry: {:?}", e);
                    None
                }
            })
            .collect(),
        Err(e) => {
            eprintln!("Error reading trace files: {}", e);
            Vec::new()
        }
    };
    paths
}
