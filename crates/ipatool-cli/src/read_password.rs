use std::io::{self, Write};

pub fn prompt_password(prompt: &str) -> io::Result<String> {
    let config = rpassword::ConfigBuilder::new()
        .password_feedback_mask('*')
        .build();
    rpassword::prompt_password_with_config(prompt, config)
}

pub fn prompt_visible(prompt: &str) -> io::Result<String> {
    let mut stderr = io::stderr();
    write!(stderr, "{prompt}")?;
    stderr.flush()?;

    let mut input = String::new();
    if io::stdin().read_line(&mut input)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "unexpected end of console input",
        ));
    }

    while input.ends_with(['\n', '\r']) {
        input.pop();
    }

    Ok(input)
}
