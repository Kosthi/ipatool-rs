use std::io;

pub fn prompt_password(prompt: &str) -> io::Result<String> {
    let config = rpassword::ConfigBuilder::new()
        .password_feedback_mask('*')
        .build();
    rpassword::prompt_password_with_config(prompt, config)
}

pub fn prompt_visible(prompt: &str) -> io::Result<String> {
    // Keep visible prompts on the controlling terminal when stdin is redirected.
    let config = rpassword::ConfigBuilder::new()
        .password_feedback_partial_mask('*', usize::MAX)
        .build();
    rpassword::prompt_password_with_config(prompt, config)
}
