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
    use std::fs;

    // ── clean_trace_file_path tests ──

    #[test]
    fn test_clean_with_cmakefiles() {
        let build_dir = Path::new("../projects/opensource/clapse/build");
        let full_path = Path::new(
            "../projects/opensource/clapse/build/lib/core/CMakeFiles/some_target.dir/src/awesome_file.cpp.json",
        );
        let result = clean_trace_file_path(full_path, build_dir).unwrap();
        assert_eq!(result, PathBuf::from("lib/core/src/awesome_file.cpp"));
    }

    #[test]
    fn test_clean_no_cmakefiles() {
        let build_dir = Path::new("/build");
        let full_path = Path::new("/build/lib/core/src/file.cpp.json");
        let result = clean_trace_file_path(full_path, build_dir).unwrap();
        assert_eq!(result, PathBuf::from("lib/core/src/file.cpp"));
    }

    #[test]
    fn test_clean_multiple_cmakefiles() {
        let build_dir = Path::new("/build");
        let full_path =
            Path::new("/build/CMakeFiles/a.dir/CMakeFiles/b.dir/src/z.cpp.json");
        let result = clean_trace_file_path(full_path, build_dir).unwrap();
        // First CMakeFiles → skip "a.dir", second CMakeFiles → skip "b.dir"
        assert_eq!(result, PathBuf::from("src/z.cpp"));
    }

    #[test]
    fn test_clean_build_dir_not_prefix() {
        let build_dir = Path::new("/build");
        let full_path = Path::new("/other/path/file.cpp.json");
        assert!(clean_trace_file_path(full_path, build_dir).is_none());
    }

    #[test]
    fn test_clean_path_equals_build_dir() {
        let build_dir = Path::new("/build");
        let full_path = Path::new("/build");
        let result = clean_trace_file_path(full_path, build_dir).unwrap();
        assert_eq!(result, PathBuf::from(""));
    }

    #[test]
    fn test_clean_no_extension() {
        let build_dir = Path::new("/build");
        let full_path = Path::new("/build/src/file");
        let result = clean_trace_file_path(full_path, build_dir).unwrap();
        assert_eq!(result, PathBuf::from("src/file"));
    }

    #[test]
    fn test_clean_hidden_file() {
        let build_dir = Path::new("/build");
        let full_path = Path::new("/build/.hidden.cpp.json");
        let result = clean_trace_file_path(full_path, build_dir).unwrap();
        assert_eq!(result, PathBuf::from(".hidden.cpp"));
    }

    // ── get_trace_files tests ──

    fn unique_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clapse_test_{}_{}", std::process::id(), name));
        // Remove leftovers from previous failed runs, then recreate
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup_temp_dir(dir: &PathBuf) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_get_trace_files_both_exist() {
        let dir = unique_temp_dir("both_exist");
        // Create subdirectory to test recursive glob
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();

        let json_path = sub.join("a.cpp.json");
        let obj_path = sub.join("a.cpp.o");
        fs::write(&json_path, "{}").unwrap();
        fs::write(&obj_path, "").unwrap();

        let result = get_trace_files(&dir);
        assert!(result.contains(&json_path), "should find a.cpp.json when .o exists");

        cleanup_temp_dir(&dir);
    }

    #[test]
    fn test_get_trace_files_json_only_no_obj() {
        let dir = unique_temp_dir("json_only");
        let json_path = dir.join("b.cpp.json");
        fs::write(&json_path, "{}").unwrap();
        // No .o file created

        let result = get_trace_files(&dir);
        assert!(!result.contains(&json_path), "should exclude json without matching .o");

        cleanup_temp_dir(&dir);
    }

    #[test]
    fn test_get_trace_files_empty_dir() {
        let dir = unique_temp_dir("empty_dir");
        let result = get_trace_files(&dir);
        assert!(result.is_empty(), "empty dir should return empty vec");

        cleanup_temp_dir(&dir);
    }
}
