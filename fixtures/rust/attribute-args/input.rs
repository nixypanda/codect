//! A multi-argument attribute wraps when it does not fit, while a short one
//! stays on a single line.

#[command(after_help = FOCUSED_DIFF_HELP, after_long_help = FOCUSED_DIFF_HELP, name = "command")]
#[derive(Debug, Clone)]
pub struct Command {
    #[arg(long = "path", short = 'p')]
    pub path: String,
}
