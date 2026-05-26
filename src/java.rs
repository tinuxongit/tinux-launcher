use anyhow::{Context, Result};
use std::path::PathBuf;
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
    let candidates = candidate_paths();
    for p in candidates {
        if let Ok(j) = probe(&p) {
            return Some(j);
        }
    }
    None
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("JAVA_HOME") {
        let exe = if cfg!(windows) { "java.exe" } else { "java" };
        out.push(PathBuf::from(p).join("bin").join(exe));
    }
    out.push(PathBuf::from(if cfg!(windows) { "java.exe" } else { "java" }));
    out
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
