#[cfg(debug_assertions)]
fn main() {
    cc_switch_lib::preview_eval::run().expect("isolated native preview evaluation");
}

#[cfg(not(debug_assertions))]
fn main() {
    eprintln!("The preview evaluation harness is debug-only.");
    std::process::exit(1);
}
