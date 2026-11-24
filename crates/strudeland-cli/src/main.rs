//! The `strudeland` command line.

use std::io::IsTerminal;
use std::process::exit;

use clap::Parser;
use strudeland_cli::cli::Cli;
use strudeland_cli::run::{Env, run};
use strudeland_cli::transport::SocketTransport;

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            exit(if e.use_stderr() { 4 } else { 0 });
        }
    };
    let mut transport = SocketTransport::new(cli.socket.clone(), cli.timeout);
    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let code = {
        let mut env = Env {
            stdin_is_tty: stdin.is_terminal(),
            stdout_is_tty: stdout.is_terminal(),
            stdin: &mut stdin,
            stdout: &mut stdout,
            stderr: &mut std::io::stderr(),
        };
        run(&cli, &mut transport, &mut env)
    };
    exit(code);
}
