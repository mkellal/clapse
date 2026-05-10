use glob::glob;
use std::path::{Path, PathBuf};

pub fn get_trace_files(build_dir: &PathBuf) -> Vec<PathBuf> {
    let pattern = format!("{}/**/*.*.json", build_dir.display());
    // let pattern = format!("{}/**/irods_configuration_parser.cpp.json", build_dir.display());   
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

pub fn clean_trace_file_path(full_path: &Path, build_dir: &Path) -> Option<PathBuf> {
    let rel_path = full_path.strip_prefix(build_dir).ok()?;

    let mut cleaned_path = PathBuf::new();
    let mut skip_next = false;

    for component in rel_path.components() {
        if skip_next {
            skip_next = false;
            continue;
        }

        if let Some(comp_str) = component.as_os_str().to_str() {
            if comp_str == "CMakeFiles" {
                skip_next = true;
                continue;
            }
        }

        cleaned_path.push(component);
    }

    cleaned_path.set_extension("");
    Some(cleaned_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_trace_file_path() {
        let build_dir = Path::new("../projects/opensource/clapse/build");
        let full_path = Path::new(
            "../projects/opensource/clapse/build/lib/core/CMakeFiles/some_target.dir/src/awesome_file.cpp.json",
        );

        let result = clean_trace_file_path(full_path, build_dir).unwrap();

        let expected = PathBuf::from("lib/core/src/awesome_file.cpp");

        assert_eq!(result, expected);
    }
}
