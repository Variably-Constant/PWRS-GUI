//! Every Rust and C program the SubEtha Guide shows, compiled into this
//! crate so its test can run each one as a process of its own, and the
//! page reader that test checks every block of the Guide with.
//!
//! `build.rs` extracts the programs: each Rust program into a module of
//! its own, each C program compiled by the host's C compiler with its
//! `main` renamed. [`run`] calls one by the page and line it is shown at.

// The C programs call the ABI; naming the crate makes the linker load its
// library alongside the C objects that reference it.
extern crate subetha_ffi;

pub mod page;

mod programs {
    include!(concat!(env!("OUT_DIR"), "/programs.rs"));
}

pub use programs::run;
