//! Which libraries an add-on leaves to the system it runs on: the C library
//! and the C++ runtime, X11, Wayland, and the Mesa and GPU drivers, which
//! must match the machine's own; the rest of WebKitGTK's closure it includes.

/// The system an add-on is built for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Platform {
    Linux,
    FreeBsd,
}

/// Library names every system that draws windows has, as patterns where
/// `*` stands for any characters.
const DRAWING: &[&str] = &[
    "libX*.so*", "libxcb*.so*", "libxkbcommon*.so*", "libxshmfence.so*",
    "libwayland-*.so*",
    "libEGL*.so*", "libGL*.so*", "libGLX*.so*", "libGLES*.so*", "libOpenGL.so*",
    "libgbm.so*", "libdrm*.so*", "libvulkan.so*",
];

/// glibc's libraries and the GCC runtime the system's own programs use.
const LINUX_RUNTIME: &[&str] = &[
    "libc.so.*", "libm.so.*", "libdl.so.*", "libpthread.so.*", "librt.so.*", "libresolv.so.*",
    "libutil.so.*", "libanl.so.*", "libnsl.so.*", "ld-linux-x86-64.so.*", "libmvec.so.*",
    "libgcc_s.so.*", "libstdc++.so.*",
];

/// Whether `name`, a library's soname, stays the system's. On FreeBSD the
/// base system's libraries, in `/lib` and `/usr/lib`, are found by the
/// caller; `in_base` says whether `name` is one of them.
pub fn is_system(name: &str, platform: Platform, in_base: bool) -> bool {
    if DRAWING.iter().any(|p| matches(p, name)) {
        return true;
    }
    match platform {
        Platform::Linux => LINUX_RUNTIME.iter().any(|p| matches(p, name)),
        Platform::FreeBsd => in_base,
    }
}

/// Whether `name` matches `pattern`, where `*` stands for any characters.
pub fn matches(pattern: &str, name: &str) -> bool {
    let mut pieces = pattern.split('*');
    let first = pieces.next().expect("split yields at least one piece");
    let Some(mut tail) = name.strip_prefix(first) else {
        return false;
    };
    let rest: Vec<&str> = pieces.collect();
    let Some((last, middle)) = rest.split_last() else {
        return tail.is_empty();
    };
    for piece in middle {
        match tail.find(piece) {
            Some(at) => tail = &tail[at + piece.len()..],
            None => return false,
        }
    }
    tail.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_as_globs() {
        assert!(matches("libX*.so*", "libX11.so.6"));
        assert!(matches("libc.so.*", "libc.so.6"));
        assert!(!matches("libc.so.*", "libcairo.so.2"));
        assert!(matches("ld-linux-x86-64.so.*", "ld-linux-x86-64.so.2"));
        assert!(!matches("libGL*.so*", "libgtk-3.so.0"));
        assert!(matches("exact", "exact"));
        assert!(!matches("exact", "exactly"));
    }

    #[test]
    fn drawing_and_runtime_stay_the_system_s_and_the_rest_rides_along() {
        assert!(is_system("libEGL.so.1", Platform::Linux, false));
        assert!(is_system("libstdc++.so.6", Platform::Linux, false));
        assert!(!is_system("libatomic.so.1", Platform::Linux, false));
        assert!(!is_system("libgtk-3.so.0", Platform::Linux, false));
        assert!(is_system("libthr.so.3", Platform::FreeBsd, true));
        assert!(!is_system("libgtk-3.so.0", Platform::FreeBsd, false));
        assert!(is_system("libwayland-client.so.0", Platform::FreeBsd, false));
    }
}
