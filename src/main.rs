fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A panic is Python's uncaught exception: its report is already printed, and
    // the interpreter still flushed stdout before exiting 1.
    let code = std::panic::catch_unwind(|| lamp::cli::main(&args)).unwrap_or(1);
    std::process::exit(lamp::out::shutdown(code));
}
