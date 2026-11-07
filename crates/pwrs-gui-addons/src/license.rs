//! What the license of each package an add-on includes requires: whether
//! the package's corresponding source must ship beside it. The LGPL, the
//! GPL, the AGPL, the MPL and Sleepycat's license (Berkeley DB's) require
//! it; a license that offers a permissive alternative does not, where the
//! package's own notice can be trusted to say so.

/// Phrases whose presence anywhere in a notice ships the source: the LGPL's
/// names, the AGPL's, and Sleepycat's license, which requires the source of
/// the database and of any software that uses it.
const SOURCE: &[&str] = &[
    "LGPL",
    "Lesser General Public",
    "Library General Public",
    "AGPL",
    "Affero General Public",
    "Sleepycat",
    "complete source code for the DB software",
];

/// What a license requires of the add-on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    /// The package's source ships in the source module.
    Source,
    /// Nothing beyond the notice.
    Notice,
    /// The text names the GPL in a form this reading cannot settle; a person
    /// has to read it and record what it means.
    Unclear,
}

/// Whether `text` names the GPL, as `GPL` standing alone or by its name.
fn names_gpl(text: &str) -> bool {
    text.contains("GNU General Public")
        || text.match_indices("GPL").any(|(at, _)| at == 0 || !text.as_bytes()[at - 1].is_ascii_alphanumeric())
}

/// Whether a license name requires source: the GPL family, the MPL and
/// Sleepycat's.
fn copyleft(name: &str) -> bool {
    name.contains("GPL") || name.starts_with("MPL") || name.eq_ignore_ascii_case("sleepycat")
}

/// The verdict on a Debian copyright file. A machine-readable file is
/// judged by the License of its `Files: *` paragraph, the package's own
/// license, where an `or` offers a choice and an `and` means each applies;
/// a free-form file that names the GPL is unclear.
pub fn debian(copyright: &str) -> Verdict {
    if SOURCE.iter().any(|phrase| copyright.contains(phrase)) {
        return Verdict::Source;
    }
    match main_license(copyright) {
        Some(main) if main.split(" or ").all(|choice| choice.split(" and ").any(copyleft)) => Verdict::Source,
        Some(_) => Verdict::Notice,
        None if names_gpl(copyright) => Verdict::Unclear,
        None => Verdict::Notice,
    }
}

/// The License of a machine-readable copyright file's `Files: *`
/// paragraph; none for a free-form file, and an empty one when the file
/// has no such paragraph.
fn main_license(copyright: &str) -> Option<String> {
    if !copyright.trim_start().starts_with("Format:") {
        return None;
    }
    for paragraph in copyright.replace("\r\n", "\n").split("\n\n") {
        let mut files = String::new();
        let mut license = None;
        let mut in_files = false;
        for line in paragraph.lines() {
            if let Some(rest) = line.strip_prefix("Files:") {
                in_files = true;
                files.push_str(rest);
            } else if in_files && line.starts_with([' ', '\t']) {
                files.push(' ');
                files.push_str(line);
            } else {
                in_files = false;
                if let Some(rest) = line.strip_prefix("License:") {
                    license.get_or_insert_with(|| rest.trim().to_string());
                }
            }
        }
        if files.split_whitespace().any(|f| f == "*")
            && let Some(license) = license
        {
            return Some(license);
        }
    }
    Some(String::new())
}

/// The verdict on a FreeBSD package's licenses as its catalog lists them.
/// A port's declaration can leave out a license some of its code is under
/// (the WebKitGTK port lists BSD2CLAUSE or GPLv2, though WebCore is LGPL),
/// so any copyleft name ships the source, whatever the combination; a
/// package with no licenses listed is unclear.
pub fn freebsd(licenses: &[String]) -> Verdict {
    if licenses.iter().all(|l| l == "NA") {
        Verdict::Unclear
    } else if licenses.iter().any(|l| copyleft(l)) {
        Verdict::Source
    } else {
        Verdict::Notice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\nUpstream-Name: x\n\n";

    #[test]
    fn a_machine_readable_file_is_judged_by_its_main_paragraph() {
        let gpl = format!("{HEADER}Files: *\nCopyright: someone\nLicense: GPL-2+\n\nFiles: debian/*\nLicense: GPL-2+\n");
        assert_eq!(debian(&gpl), Verdict::Source);
        let choice = format!("{HEADER}Files: *\nLicense: BSD-3-clause or GPL-2\n");
        assert_eq!(debian(&choice), Verdict::Notice);
        let both = format!("{HEADER}Files: *\nLicense: BSD-3-clause and GPL-2\n");
        assert_eq!(debian(&both), Verdict::Source);
        let tools = format!("{HEADER}Files: *\nLicense: Zlib\n\nFiles: config.guess\n config.sub\nLicense: GPL-3+ with autoconf exception\n");
        assert_eq!(debian(&tools), Verdict::Notice);
        let listed = format!("{HEADER}Files: src/*\n *\nLicense: MPL-2.0\n");
        assert_eq!(debian(&listed), Verdict::Source);
        let lesser = format!("{HEADER}Files: *\nLicense: LGPL-2.1+\n");
        assert_eq!(debian(&lesser), Verdict::Source);
    }

    #[test]
    fn a_free_form_file_is_judged_by_what_it_names() {
        assert_eq!(debian("This library is under the GNU General Public License."), Verdict::Unclear);
        assert_eq!(debian("Released under the GPL, version 2."), Verdict::Unclear);
        assert_eq!(debian("Permission is hereby granted, free of charge"), Verdict::Notice);
        assert_eq!(debian("Copyright Oracle. complete source code for the DB software"), Verdict::Source);
        assert_eq!(debian("GNU Library General Public License version 2"), Verdict::Source);
        assert_eq!(debian("Released under the AGPL"), Verdict::Source);
    }

    #[test]
    fn a_freebsd_copyleft_name_ships_source_whatever_the_combination() {
        let names = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(freebsd(&names(&["BSD2CLAUSE", "GPLv2"])), Verdict::Source);
        assert_eq!(freebsd(&names(&["LGPL20"])), Verdict::Source);
        assert_eq!(freebsd(&names(&["MIT"])), Verdict::Notice);
        assert_eq!(freebsd(&names(&["APACHE20", "MIT", "MPL20"])), Verdict::Source);
        assert_eq!(freebsd(&[]), Verdict::Unclear);
        assert_eq!(freebsd(&names(&["NA"])), Verdict::Unclear);
    }
}
