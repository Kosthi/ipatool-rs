use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::tui::app::{App_, InputMode};
use crate::tui::theme;

pub fn render(f: &mut Frame, app: &mut App_, area: Rect) {
    let bg = Block::default().style(theme::base());
    f.render_widget(bg, area);

    if let Some(ref account) = app.account {
        render_logged_in(f, account, area);
    } else {
        render_login_form(f, app, area);
    }
}

fn render_logged_in(f: &mut Frame, account: &ipatool_core::model::Account, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Logged In",
            theme::success_style().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        detail_line("  Name", &account.name),
        detail_line("  Email", &account.email),
        detail_line("  DSID", &account.directory_services_id),
        detail_line("  Store", &account.store_front),
        detail_line("  Pod", account.pod.as_deref().unwrap_or("—")),
        Line::from(""),
        Line::from(vec![
            Span::styled("  r", theme::tab_active()),
            Span::styled(" revoke credentials", theme::label()),
        ]),
    ];

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn render_login_form(f: &mut Frame, app: &App_, area: Rect) {
    let email_active = app.input_mode == InputMode::LoginEmail;
    let pass_active = app.input_mode == InputMode::LoginPassword;
    let auth_code_active = app.input_mode == InputMode::LoginAuthCode;

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Span::styled("  Login", theme::title())),
        chunks[0],
    );

    let email_label_style = if email_active {
        theme::border_active()
    } else {
        theme::label()
    };
    f.render_widget(
        Paragraph::new(Span::styled("  Email", email_label_style)),
        chunks[1],
    );

    let email_text = if app.login_email.value().is_empty() && !email_active {
        Line::from(Span::styled("  your@email.com", theme::input_placeholder()))
    } else {
        Line::from(Span::styled(
            format!("  {}", app.login_email.value()),
            theme::input_active(),
        ))
    };
    let email_border = if email_active {
        theme::border_active()
    } else {
        theme::border_dim()
    };
    let email = Paragraph::new(email_text).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(email_border),
    );
    f.render_widget(email, chunks[2]);

    if email_active {
        let cx = chunks[2].x + app.login_email.visual_cursor() as u16 + 3;
        let cy = chunks[2].y;
        f.set_cursor_position((cx, cy));
    }

    let pass_label_style = if pass_active {
        theme::border_active()
    } else {
        theme::label()
    };
    f.render_widget(
        Paragraph::new(Span::styled("  Password", pass_label_style)),
        chunks[3],
    );

    let masked: String = if app.login_password.is_empty() && !pass_active {
        String::new()
    } else {
        "*".repeat(app.login_password.len())
    };
    let pass_display = if masked.is_empty() && !pass_active {
        Line::from(Span::styled("  ••••••••", theme::input_placeholder()))
    } else {
        Line::from(Span::styled(format!("  {masked}"), theme::input_active()))
    };
    let pass_border = if pass_active {
        theme::border_active()
    } else {
        theme::border_dim()
    };
    let password = Paragraph::new(pass_display).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(pass_border),
    );
    f.render_widget(password, chunks[4]);

    if pass_active {
        let cx = chunks[4].x + app.login_password.len() as u16 + 3;
        let cy = chunks[4].y;
        f.set_cursor_position((cx, cy));
    }

    let auth_code_label_style = if auth_code_active {
        theme::border_active()
    } else {
        theme::label()
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            "  Two-factor code (optional)",
            auth_code_label_style,
        )),
        chunks[5],
    );

    let auth_code_text = if app.login_auth_code.value().is_empty() && !auth_code_active {
        Line::from(Span::styled("  123456", theme::input_placeholder()))
    } else {
        Line::from(Span::styled(
            format!("  {}", app.login_auth_code.value()),
            theme::input_active(),
        ))
    };
    let auth_code_border = if auth_code_active {
        theme::border_active()
    } else {
        theme::border_dim()
    };
    let auth_code = Paragraph::new(auth_code_text).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(auth_code_border),
    );
    f.render_widget(auth_code, chunks[6]);

    if auth_code_active {
        let cx = chunks[6].x + app.login_auth_code.visual_cursor() as u16 + 3;
        let cy = chunks[6].y;
        f.set_cursor_position((cx, cy));
    }

    let help = if let Some(ref err) = app.login_error {
        vec![Line::from(Span::styled(
            format!("  {err}"),
            theme::error_style(),
        ))]
    } else if email_active || pass_active || auth_code_active {
        vec![Line::from(Span::styled(
            "  Tab: next field  Enter: submit  Esc: cancel",
            theme::label(),
        ))]
    } else {
        vec![
            Line::from(""),
            Line::from(Span::styled("  Press l to log in", theme::label())),
        ]
    };

    f.render_widget(Paragraph::new(help), chunks[7]);

    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Credentials are stored in the system keychain.",
                theme::label(),
            )),
        ]),
        chunks[8],
    );
}

fn detail_line<'a>(label: &'a str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label}: "), theme::label()),
        Span::styled(value, theme::value()),
    ])
}
