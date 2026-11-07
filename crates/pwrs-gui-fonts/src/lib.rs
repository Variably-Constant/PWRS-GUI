//! The fonts PWRS-GUI draws text in, included in its build so that both
//! renderers draw the same faces on every system: Inter for text and
//! JetBrains Mono's cut without ligatures for monospace text, so `!=`, `->`
//! and `>=` show as typed. Each file is unchanged from its project's
//! release, and each is under the SIL Open Font License 1.1, whose texts,
//! with each font's copyright, are this crate's `LICENSE-Inter.txt` and
//! `LICENSE-JetBrainsMono.txt`.
//!
//! | File | From | SHA-256 |
//! |---|---|---|
//! | `fonts/InterVariable.ttf` | Inter 4.1's `Inter-4.1.zip` | `4989b125924991b90d05b2d16e0e388c48f7d5bb8b30539bbf9c755278d0ccaf` |
//! | `fonts/JetBrainsMonoNL-Regular.ttf` | JetBrains Mono 2.304's `JetBrainsMono-2.304.zip` | `fb3b2575d7b0657359707993288f12a7360344d39387bb26050e276d61f6bd2a` |

/// Inter's variable face, whose default instance, its Regular at its text
/// optical size, is the one egui draws and a page draws with optical sizing
/// off.
pub const INTER: &[u8] = include_bytes!("../fonts/InterVariable.ttf");

/// The family Inter's face names itself.
pub const INTER_FAMILY: &str = "Inter Variable";

/// JetBrains Mono's Regular face without ligatures.
pub const JETBRAINS_MONO: &[u8] = include_bytes!("../fonts/JetBrainsMonoNL-Regular.ttf");

/// The family JetBrains Mono's face without ligatures names itself.
pub const JETBRAINS_MONO_FAMILY: &str = "JetBrains Mono NL";
