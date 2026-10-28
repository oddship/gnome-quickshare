use gtk::glib;
use std::ffi::{CString, c_char};
use std::path::PathBuf;

const DOMAIN: &str = "gnome-quickshare";

unsafe extern "C" {
    fn bindtextdomain(domain: *const c_char, directory: *const c_char) -> *mut c_char;
    fn bind_textdomain_codeset(domain: *const c_char, codeset: *const c_char) -> *mut c_char;
}

/// Called once on the main thread before GTK, logging or worker threads start.
pub fn init() {
    let directory = std::env::var_os("GNOME_QUICKSHARE_LOCALEDIR")
        .map(PathBuf::from)
        .or_else(|| {
            let executable = std::env::current_exe().ok()?;
            let directory = executable.parent()?.parent()?.join("share/locale");
            directory.is_dir().then_some(directory)
        })
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("po/locale"));
    let Ok(directory) = CString::new(directory.as_os_str().as_encoded_bytes()) else {
        return;
    };
    // These process-global locale functions run before any threads are created.
    // gettext copies the domain/path; all C strings remain live during each call.
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
        bindtextdomain(c"gnome-quickshare".as_ptr(), directory.as_ptr());
        bind_textdomain_codeset(c"gnome-quickshare".as_ptr(), c"UTF-8".as_ptr());
    }
}

pub fn tr(message: &str) -> String {
    if message.is_empty() {
        return String::new();
    }
    glib::dgettext(Some(DOMAIN), message).into()
}

/// Mark a stored label for extraction, translating it only when displayed.
pub const fn mark(message: &str) -> &str {
    message
}

/// Named placeholders let translators reorder complete sentences.
pub fn trf(message: &str, arguments: &[(&str, &str)]) -> String {
    let translated = tr(message);
    substitute(&translated, arguments)
}

pub fn ntr(singular: &str, plural: &str, count: usize) -> String {
    let translated = glib::dngettext(Some(DOMAIN), singular, plural, count as _);
    substitute(&translated, &[("count", &count.to_string())])
}

fn substitute(message: &str, arguments: &[(&str, &str)]) -> String {
    // Scan the template once: braces inside filenames or received text are data,
    // and must never be expanded as another translated placeholder.
    let mut result = String::new();
    let mut remaining = message;
    while let Some(start) = remaining.find('{') {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        let Some(end) = remaining.find('}') else {
            break;
        };
        let name = &remaining[1..end];
        if let Some((_, value)) = arguments.iter().find(|(key, _)| *key == name) {
            result.push_str(value);
        } else {
            result.push_str(&remaining[..=end]);
        }
        remaining = &remaining[end + 1..];
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reordered_placeholders_preserve_untrusted_text() {
        assert_eq!(
            substitute(
                "{device}: {file}",
                &[("file", "{device}.txt"), ("device", "Laptop")]
            ),
            "Laptop: {device}.txt"
        );
        assert_eq!(substitute("{unknown} {", &[]), "{unknown} {");
    }
}
