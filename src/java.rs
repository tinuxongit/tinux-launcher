use anyhow::{Context, Result};
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct JavaInstall {
    pub path: PathBuf,
    pub major: u32,
}

impl JavaInstall {
    pub fn launch_path(&self) -> PathBuf {
        #[cfg(windows)]
        {
            if self.path.file_name().and_then(|s| s.to_str()) == Some("java.exe") {
                let javaw = self.path.with_file_name("javaw.exe");
                if javaw.exists() {
                    return javaw;
                }
            }
        }
        self.path.clone()
    }
}

pub fn detect_default() -> Option<JavaInstall> {
    detect_all().into_iter().next()
}

pub fn detect_for_major(major: u32) -> Option<JavaInstall> {
    detect_all().into_iter().find(|j| j.major == major)
}

pub fn detect_all() -> Vec<JavaInstall> {
    candidate_paths()
        .into_iter()
        .filter_map(|p| probe(&p).ok())
        .collect()
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let exe = if cfg!(windows) { "java.exe" } else { "java" };

    if let Ok(p) = env::var("JAVA_HOME") {
        push_candidate(&mut out, &mut seen, PathBuf::from(p).join("bin").join(exe));
    }

    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            push_candidate(&mut out, &mut seen, dir.join(exe));
        }
    }

    #[cfg(windows)]
    {
        if let Ok(output) = Command::new("where.exe").arg("java").output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().map(str::trim).filter(|line| !line.is_empty()) {
                push_candidate(&mut out, &mut seen, PathBuf::from(line));
            }
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Ok(root) = env::var(var) {
                push_children(
                    PathBuf::from(&root).join("Java"),
                    exe,
                    &mut out,
                    &mut seen,
                );
                push_children(
                    PathBuf::from(&root).join("Eclipse Adoptium"),
                    exe,
                    &mut out,
                    &mut seen,
                );
            }
        }
        if let Ok(user_profile) = env::var("USERPROFILE") {
            let apps = PathBuf::from(user_profile).join("scoop").join("apps");
            if let Ok(entries) = std::fs::read_dir(apps) {
                for entry in entries.flatten() {
                    push_candidate(
                        &mut out,
                        &mut seen,
                        entry.path().join("current").join("bin").join(exe),
                    );
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("/usr/libexec/java_home").arg("-V").output() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            for line in stderr.lines() {
                if let Some(home) = java_home_from_macos_line(line) {
                    push_candidate(&mut out, &mut seen, home.join("bin").join(exe));
                }
            }
        }
        push_children(
            PathBuf::from("/Library/Java/JavaVirtualMachines"),
            "Contents/Home/bin/java",
            &mut out,
            &mut seen,
        );
        push_children(
            PathBuf::from("/System/Library/Java/JavaVirtualMachines"),
            "Contents/Home/bin/java",
            &mut out,
            &mut seen,
        );
        if let Ok(home) = env::var("HOME") {
            let home = PathBuf::from(home);
            push_children(
                home.join(".sdkman").join("candidates").join("java"),
                "bin/java",
                &mut out,
                &mut seen,
            );
        }
        for homebrew in ["/opt/homebrew/opt", "/usr/local/opt"] {
            for formula in ["openjdk", "openjdk@8", "openjdk@11", "openjdk@17", "openjdk@21"] {
                push_candidate(
                    &mut out,
                    &mut seen,
                    PathBuf::from(homebrew).join(formula).join("bin").join(exe),
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        push_children(PathBuf::from("/usr/lib/jvm"), "bin/java", &mut out, &mut seen);
        push_children(PathBuf::from("/usr/java"), "bin/java", &mut out, &mut seen);
        push_children(PathBuf::from("/opt/java"), "bin/java", &mut out, &mut seen);
        if let Ok(home) = env::var("HOME") {
            let home = PathBuf::from(home);
            push_children(
                home.join(".sdkman").join("candidates").join("java"),
                "bin/java",
                &mut out,
                &mut seen,
            );
            push_children(home.join(".jabba").join("jdk"), "bin/java", &mut out, &mut seen);
        }
    }

    if out.is_empty() {
        push_candidate(&mut out, &mut seen, PathBuf::from(exe));
    }
    out
}

fn push_candidate(out: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: PathBuf) {
    let key = path.to_string_lossy().to_lowercase();
    if seen.insert(key) {
        out.push(path);
    }
}

fn push_children(root: PathBuf, java_subpath: &str, out: &mut Vec<PathBuf>, seen: &mut HashSet<String>) {
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            push_candidate(out, seen, entry.path().join(Path::new(java_subpath)));
        }
    }
}

#[cfg(target_os = "macos")]
fn java_home_from_macos_line(line: &str) -> Option<PathBuf> {
    let start = line.find("/Library/Java/JavaVirtualMachines/")?;
    Some(PathBuf::from(line[start..].trim()))
}

pub fn probe(path: &std::path::Path) -> Result<JavaInstall> {
    let output = Command::new(path)
        .arg("-version")
        .output()
        .with_context(|| format!("running {}", path.display()))?;
    // java -version writes to stderr
    let s = String::from_utf8_lossy(&output.stderr);
    let major = parse_major(&s).context("parsing java -version output")?;
    Ok(JavaInstall {
        path: path.to_path_buf(),
        major,
    })
}

fn parse_major(s: &str) -> Option<u32> {
    let first_quote = s.find('"')?;
    let rest = &s[first_quote + 1..];
    let end = rest.find('"')?;
    let ver = &rest[..end];
    let parts: Vec<&str> = ver.split('.').collect();
    if parts.is_empty() {
        return None;
    }
    let first = parts[0].parse::<u32>().ok()?;
    if first == 1 && parts.len() > 1 {
        parts[1].parse::<u32>().ok()
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_major;

    #[test]
    fn jdk8() {
        let s = "java version \"1.8.0_491\"\nJava(TM) SE Runtime Environment ...";
        assert_eq!(parse_major(s), Some(8));
    }

    #[test]
    fn jdk17() {
        let s = "openjdk version \"17.0.9\" 2023-10-17 LTS";
        assert_eq!(parse_major(s), Some(17));
    }

    #[test]
    fn jdk21() {
        let s = "openjdk version \"21\" 2023-09-19";
        assert_eq!(parse_major(s), Some(21));
    }
}
