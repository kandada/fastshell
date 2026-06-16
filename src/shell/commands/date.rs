use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_date(&self, args: &[&str]) -> CommandOutput {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();

        let format = if args.is_empty() {
            None
        } else {
            let fmt = args.join(" ");
            if fmt.starts_with('+') {
                Some(fmt[1..].to_string())
            } else {
                Some(fmt)
            }
        };

        let output = match format {
            Some(ref fmt) => format_date(secs, fmt),
            None => format!("{}\n", crate::shell::format_unix_time(secs)),
        };

        CommandOutput::success(output)
    }
}

fn format_date(secs: u64, fmt: &str) -> String {
    let days_since_epoch = (secs / 86400) as i32;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = crate::shell::civil_from_days(days_since_epoch);

    let mut result = fmt.to_string();
    result = result.replace("%Y", &format!("{:04}", year));
    result = result.replace("%m", &format!("{:02}", month));
    result = result.replace("%d", &format!("{:02}", day));
    result = result.replace("%H", &format!("{:02}", hours));
    result = result.replace("%M", &format!("{:02}", minutes));
    result = result.replace("%S", &format!("{:02}", seconds));
    result.push('\n');
    result
}
