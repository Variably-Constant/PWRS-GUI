//! The folder a WebKitGTK library has compiled in for its own processes,
//! rewritten in place to a path of the same length relative to the folder a
//! process runs from, since current WebKitGTK reads `WEBKIT_EXEC_PATH` only
//! in developer builds.

/// Replaces every `from` in `data` with `to`, which must be as long, and
/// returns how many it replaced.
pub fn same_length(data: &mut [u8], from: &[u8], to: &[u8]) -> Result<usize, String> {
    if from.len() != to.len() {
        return Err(format!(
            "{:?} and {:?} differ in length ({} and {}), so one cannot stand in for the other in a library",
            String::from_utf8_lossy(from),
            String::from_utf8_lossy(to),
            from.len(),
            to.len()
        ));
    }
    if from.is_empty() {
        return Err(String::from("an empty string cannot be rewritten"));
    }
    let mut count = 0;
    let mut at = 0;
    while at + from.len() <= data.len() {
        if &data[at..at + from.len()] == from {
            data[at..at + from.len()].copy_from_slice(to);
            count += 1;
            at += from.len();
        } else {
            at += 1;
        }
    }
    Ok(count)
}

/// The relative form of an absolute `prefix`: its first `/usr/` becomes
/// `././/`, so `/usr/lib/x` reads `lib/x` from the working folder.
pub fn relative(prefix: &str) -> Result<String, String> {
    match prefix.strip_prefix("/usr/") {
        Some(rest) => Ok(format!("././/{rest}")),
        None => Err(format!("{prefix} does not start with /usr/, the one prefix this rewrite turns relative")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_occurrence_is_replaced_at_the_same_length() {
        let mut data = b"x/usr/lib/a\0y/usr/lib/a/b\0".to_vec();
        let to = relative("/usr/lib/a").expect("relative");
        assert_eq!(to, "././/lib/a");
        assert_eq!(same_length(&mut data, b"/usr/lib/a", to.as_bytes()).expect("rewritten"), 2);
        assert_eq!(&data, b"x././/lib/a\0y././/lib/a/b\0");
    }

    #[test]
    fn a_different_length_or_prefix_is_refused() {
        let mut data = b"/usr/lib".to_vec();
        assert!(same_length(&mut data, b"/usr/lib", b"lib").is_err());
        assert!(relative("/opt/lib").is_err());
    }
}
