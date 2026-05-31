#[cfg(windows)]
fn main() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/tinux-icon.ico");
    res.set("FileDescription", "Tinux Launcher");
    res.set("ProductName", "Tinux Launcher");
    res.set("InternalName", "tinux-launcher");
    res.set("OriginalFilename", "tinux-launcher.exe");
    res.compile().expect("compile Windows resources");
}

#[cfg(not(windows))]
fn main() {}
