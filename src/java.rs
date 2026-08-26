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

pub fn detect_at_path(path: &Path) -> Option<JavaInstall> {
    probe(path).ok()
}

pub fn detect_for_major(major: u32) -> Option<JavaInstall> {
    detect_all().into_iter().find(|j| j.major == major)
}

/// Whether an installed JVM can run a version that asks for `required`.
///
/// Mojang names one major version per release, but a newer JVM runs those
/// releases fine: 1.20.1 asks for 17 and plays on 21. Versions from the Java 8
/// era are the exception — they reach into JDK internals that modern JVMs
/// removed — so those take the exact major only.
pub fn major_can_run(installed: u32, required: u32) -> bool {
    if required <= 8 {
        installed == required
    } else {
        installed >= required
    }
}

/// The best installed JVM for a version: the exact major the version asks for
/// when it's here, otherwise the closest newer one.
pub fn detect_for_version(required: u32) -> Option<JavaInstall> {
    best_for_version(detect_all(), required)
}

/// Pick the closest usable JVM, which is the lowest one that can run it.
///
/// Newer is not automatically better. Mod loaders rewrite bytecode as they
/// load it, and Fabric's Mixin refuses class files from a Java it doesn't
/// know, so a 1.20.1 pack asking for 17 has to get 17 on a machine that also
/// has 21 and 25 sitting there.
fn best_for_version(installs: Vec<JavaInstall>, required: u32) -> Option<JavaInstall> {
    let mut usable: Vec<JavaInstall> = installs
        .into_iter()
        .filter(|j| major_can_run(j.major, required))
        .collect();
    usable.sort_by_key(|j| j.major);
    usable.into_iter().next()
}

/// Every distinct Java on the machine, best candidate first.
///
/// Deduplicated by where the executable really lives, not by the path used to
/// reach it. On Linux one JDK is typically reachable as `/usr/bin/java`,
/// `/bin/java`, `/usr/lib/jvm/default/bin/java` and more, and listing it five
/// times makes an error message look like a wall of installs.
pub fn detect_all() -> Vec<JavaInstall> {
    let mut seen = HashSet::new();
    candidate_paths()
        .into_iter()
        .filter(|p| {
            let real = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            seen.insert(real)
        })
        .filter_map(|p| probe(&p).ok())
        .collect()
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let exe = if cfg!(windows) { "java.exe" } else { "java" };

    // Runtimes this launcher downloaded from Mojang come first: they were
    // fetched because a version asked for exactly that Java.
    if let Some(dirs) = directories::ProjectDirs::from("dev", "tinux", "TinuxLauncher") {
        push_children(
            dirs.data_dir().join("runtimes"),
            &format!("bin/{exe}"),
            &mut out,
            &mut seen,
        );
    }

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
        for root in [
            "/usr/lib/jvm",
            "/usr/lib64/jvm",
            "/usr/java",
            "/opt/java",
            // Distro packages and hand-unpacked JDKs both land in /opt.
            "/opt",
            // Flatpak and Snap runtimes.
            "/var/lib/flatpak/runtime",
            "/snap",
        ] {
            push_children(PathBuf::from(root), "bin/java", &mut out, &mut seen);
        }
        if let Ok(home) = env::var("HOME") {
            let home = PathBuf::from(home);
            push_children(
                home.join(".sdkman").join("candidates").join("java"),
                "bin/java",
                &mut out,
                &mut seen,
            );
            push_children(home.join(".jabba").join("jdk"), "bin/java", &mut out, &mut seen);
            // JDKs other launchers and build tools manage for themselves.
            for root in [
                ".jdks",
                ".gradle/jdks",
                ".minecraft/runtime",
                ".local/share/PrismLauncher/java",
                ".local/lib/jvm",
            ] {
                push_children(home.join(root), "bin/java", &mut out, &mut seen);
            }
            // The official launcher nests one more level:
            // runtime/<component>/<platform>/<component>/bin/java.
            push_grandchildren(
                home.join(".minecraft").join("runtime"),
                "bin/java",
                &mut out,
                &mut seen,
            );
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

/// `push_children` two levels down, for layouts that nest a platform folder
/// between the root and the JDK.
#[allow(dead_code)]
fn push_grandchildren(
    root: PathBuf,
    java_subpath: &str,
    out: &mut Vec<PathBuf>,
    seen: &mut HashSet<String>,
) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let Ok(inner) = std::fs::read_dir(entry.path()) else { continue };
        for child in inner.flatten() {
            push_children(child.path(), java_subpath, out, seen);
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
    use super::{best_for_version, major_can_run, parse_major, JavaInstall};

    fn installs(majors: &[u32]) -> Vec<JavaInstall> {
        majors
            .iter()
            .map(|m| JavaInstall {
                path: format!("/jvm/{m}/bin/java").into(),
                major: *m,
            })
            .collect()
    }

    #[test]
    fn picks_the_closest_java_not_the_newest() {
        // A 1.20.1 pack asks for 17: Mixin breaks on 25, so 17 has to win.
        let chosen = best_for_version(installs(&[25, 21, 17]), 17).unwrap();
        assert_eq!(chosen.major, 17);
    }

    #[test]
    fn falls_forward_when_the_exact_major_is_absent() {
        let chosen = best_for_version(installs(&[25, 21]), 17).unwrap();
        assert_eq!(chosen.major, 21);
        assert!(best_for_version(installs(&[21, 25]), 26).is_none());
    }

    #[test]
    fn newer_java_runs_modern_versions() {
        // 1.20.1 asks for 17; 21 is what most Linux distros ship.
        assert!(major_can_run(21, 17));
        assert!(major_can_run(17, 17));
        assert!(!major_can_run(11, 17));
    }

    #[test]
    fn old_versions_need_their_own_java() {
        assert!(major_can_run(8, 8));
        assert!(!major_can_run(21, 8));
    }

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
