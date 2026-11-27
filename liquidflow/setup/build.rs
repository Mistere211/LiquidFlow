fn main() {
    #[cfg(windows)]
    {
        let mut r = winresource::WindowsResource::new();
        r.set_icon("assets/icon.ico");
        let _ = r.compile();
    }
}
