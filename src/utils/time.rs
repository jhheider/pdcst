use chrono::{DateTime, Local, Utc};

pub fn format_duration(seconds: i64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

pub fn format_date(dt: &DateTime<Utc>) -> String {
    let local: DateTime<Local> = DateTime::from(*dt);
    local.format("%Y-%m-%d %H:%M").to_string()
}

pub fn format_relative_time(dt: &DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(*dt);

    let days = duration.num_days();
    let hours = duration.num_hours();
    let minutes = duration.num_minutes();

    if days > 365 {
        let years = days / 365;
        format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
    } else if days > 30 {
        let months = days / 30;
        format!("{} month{} ago", months, if months == 1 { "" } else { "s" })
    } else if days > 0 {
        format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
    } else if hours > 0 {
        format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
    } else if minutes > 0 {
        format!(
            "{} minute{} ago",
            minutes,
            if minutes == 1 { "" } else { "s" }
        )
    } else {
        "Just now".to_string()
    }
}
