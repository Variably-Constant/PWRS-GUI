//! What every add-on's module manifest shares.

/// A GUID that stays the same for a module's name, from its bytes.
pub fn guid(name: &str) -> String {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x8422_2325_cbf2_9ce4;
    for byte in name.bytes() {
        a = (a ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        b = (b ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(7);
    }
    let h = format!("{a:016x}{b:016x}");
    format!("{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}
