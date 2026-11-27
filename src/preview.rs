//! Tab-Vorschau (nur Windows, Feature "previews"): WebView2 CapturePreview -> PNG-Bytes.
use webview2_com::CapturePreviewCompletedHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
use windows::Win32::System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};
use windows::Win32::Foundation::{BOOL, HGLOBAL};
use wry::WebViewExtWindows;

pub fn capture(view: &wry::WebView, done: impl FnOnce(Vec<u8>) + 'static) {
    unsafe {
        let Ok(core) = view.controller().CoreWebView2() else { return };
        let Ok(stream) = CreateStreamOnHGlobal::<HGLOBAL, BOOL>(HGLOBAL::default(), BOOL::from(true)) else { return };
        let s2 = stream.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |res| {
            if res.is_ok() {
                let mut st = STATSTG::default();
                if s2.Stat(&mut st, STATFLAG_NONAME).is_ok() {
                    let len = st.cbSize as usize;
                    let _ = s2.Seek(0, STREAM_SEEK_SET, None);
                    let mut buf = vec![0u8; len];
                    let mut read = 0u32;
                    let _ = s2.Read(buf.as_mut_ptr() as *mut _, len as u32, Some(&mut read));
                    buf.truncate(read as usize);
                    if !buf.is_empty() { done(buf); }
                }
            }
            Ok(())
        }));
        let _ = core.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler);
    }
}
