//! The daily-note calendar: a month grid where the days that have a note are marked.
//!
//! Port of the upstream `utils/calendar.ts` and the popover. A daily note is a note named
//! `YYYY-MM-DD`; clicking a day opens it, creating it when missing (with `_templates/daily.md` when
//! the user keeps one). The grid is six weeks long so it never changes height between months.

use chrono::{Datelike, Local, NaiveDate};

/// One day of the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Day {
    /// `YYYY-MM-DD`.
    pub iso: String,
    pub number: u32,
    /// False for the days that pad the grid from the neighbouring months.
    pub in_month: bool,
}

/// `YYYY-MM-DD` for a date.
pub fn to_iso(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Today, `YYYY-MM-DD`.
pub fn today_iso() -> String {
    to_iso(Local::now().date_naive())
}

/// Parses `YYYY-MM-DD` as local midnight, for the template placeholders of another day.
pub fn local_midnight(iso: &str) -> Option<chrono::DateTime<Local>> {
    let date = from_iso(iso)?;
    date.and_hms_opt(0, 0, 0)?.and_local_timezone(Local).single()
}

/// Parses `YYYY-MM-DD`.
pub fn from_iso(iso: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d").ok()
}

/// Whether a note's name is a daily note.
pub fn is_daily_note_name(name: &str) -> bool {
    name.len() == 10 && from_iso(name).is_some()
}

/// Adds months, clamping the day so 31 January + 1 month is the end of February.
pub fn add_months(iso: &str, delta: i32) -> String {
    let Some(base) = from_iso(iso) else { return iso.to_string() };
    let month = base.month0() as i32 + delta;
    let year = base.year() + month.div_euclid(12);
    let month = month.rem_euclid(12) as u32 + 1;
    let last = last_day(year, month);
    let day = base.day().min(last);
    NaiveDate::from_ymd_opt(year, month, day).map(to_iso).unwrap_or_else(|| iso.to_string())
}

fn last_day(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(next_year, next_month, 1).and_then(|date| date.pred_opt()).map_or(28, |date| date.day())
}

/// Six weeks of days covering the month `iso` falls in, padded with the neighbouring months.
/// `week_start` is 0 for Sunday, 1 for Monday.
pub fn month_grid(iso: &str, week_start: u32) -> Vec<Day> {
    let Some(base) = from_iso(iso) else { return Vec::new() };
    let first = NaiveDate::from_ymd_opt(base.year(), base.month(), 1).unwrap_or(base);
    let monday_based = first.weekday().num_days_from_monday();
    let start_weekday = if week_start == 0 { (monday_based + 1) % 7 } else { monday_based };
    let start = first - chrono::Duration::days(i64::from(start_weekday));

    (0..42)
        .map(|offset| {
            let date = start + chrono::Duration::days(offset);
            Day { iso: to_iso(date), number: date.day(), in_month: date.month() == base.month() }
        })
        .collect()
}

/// Month name plus year, in the interface's language.
pub fn month_label(iso: &str, language: &str) -> String {
    let Some(date) = from_iso(iso) else { return iso.to_string() };
    let months = month_names(language);
    let name = months.get(date.month0() as usize).copied().unwrap_or("");
    format!("{name} {}", date.year())
}

/// Weekday initials in display order, starting at `week_start`.
pub fn weekday_labels(language: &str, week_start: u32) -> Vec<&'static str> {
    let names = weekday_names(language);
    (0..7).map(|index| names[((index + week_start) % 7) as usize]).collect()
}

fn month_names(language: &str) -> [&'static str; 12] {
    match language {
        "pt-br" => ["Janeiro", "Fevereiro", "Março", "Abril", "Maio", "Junho", "Julho", "Agosto", "Setembro", "Outubro", "Novembro", "Dezembro"],
        "es" => ["Enero", "Febrero", "Marzo", "Abril", "Mayo", "Junio", "Julio", "Agosto", "Septiembre", "Octubre", "Noviembre", "Diciembre"],
        "ja" => ["1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月"],
        _ => ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
    }
}

fn weekday_names(language: &str) -> [&'static str; 7] {
    match language {
        "pt-br" => ["D", "S", "T", "Q", "Q", "S", "S"],
        "es" => ["D", "L", "M", "M", "J", "V", "S"],
        "ja" => ["日", "月", "火", "水", "木", "金", "土"],
        _ => ["S", "M", "T", "W", "T", "F", "S"],
    }
}

/// Draws the month `month` (a `YYYY-MM-DD` inside it). Returns the day the user picked.
pub fn show(ui: &mut egui::Ui, month: &mut String, marked: &dyn Fn(&str) -> bool, language: &str) -> Option<String> {
    let mut picked = None;
    ui.horizontal(|ui| {
        if ui.small_button("<").on_hover_text(crate::tl!("Previous month")).clicked() {
            *month = add_months(month, -1);
        }
        ui.label(egui::RichText::new(month_label(month, language)).strong());
        if ui.small_button(">").on_hover_text(crate::tl!("Next month")).clicked() {
            *month = add_months(month, 1);
        }
        if ui.small_button(crate::tl!("Today")).on_hover_text(crate::tl!("Back to this month")).clicked() {
            *month = today_iso();
        }
    });
    ui.add_space(4.0);

    let today = today_iso();
    egui::Grid::new("calendar-grid").num_columns(7).spacing([4.0, 4.0]).show(ui, |ui| {
        for weekday in weekday_labels(language, 1) {
            ui.label(egui::RichText::new(weekday).small().weak());
        }
        ui.end_row();

        for (index, day) in month_grid(month, 1).iter().enumerate() {
            let has_note = marked(&day.iso);
            let is_today = day.iso == today;
            let text = if has_note {
                egui::RichText::new(day.number.to_string()).strong()
            } else if day.in_month {
                egui::RichText::new(day.number.to_string())
            } else {
                egui::RichText::new(day.number.to_string()).weak()
            };
            let text = if is_today { text.underline() } else { text };
            let button = egui::Button::new(text).min_size(egui::vec2(28.0, 24.0));
            let response = ui.add(button);
            let response = if has_note { response.on_hover_text(crate::tl!("Open this day's note")) } else { response };
            if response.clicked() {
                picked = Some(day.iso.clone());
            }
            if index % 7 == 6 {
                ui.end_row();
            }
        }
    });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_round_trips_through_iso() {
        assert_eq!(to_iso(from_iso("2026-07-21").expect("valid")), "2026-07-21");
        assert!(from_iso("21/07/2026").is_none());
    }

    #[test]
    fn a_daily_note_is_named_after_its_date() {
        assert!(is_daily_note_name("2026-07-21"));
        assert!(!is_daily_note_name("2026-7-21"));
        assert!(!is_daily_note_name("Sepse"));
    }

    #[test]
    fn adding_a_month_clamps_the_day() {
        assert_eq!(add_months("2026-01-31", 1), "2026-02-28");
        assert_eq!(add_months("2026-03-31", -1), "2026-02-28");
        assert_eq!(add_months("2026-12-15", 1), "2027-01-15");
        assert_eq!(add_months("2028-01-31", 1), "2028-02-29", "leap year");
    }

    #[test]
    fn the_grid_is_six_weeks_and_starts_on_monday() {
        let grid = month_grid("2026-07-21", 1);
        assert_eq!(grid.len(), 42);
        // 2026-07-01 is a Wednesday, so the grid starts on Monday 2026-06-29.
        assert_eq!(grid[0].iso, "2026-06-29");
        assert!(!grid[0].in_month);
        assert_eq!(grid.iter().filter(|day| day.in_month).count(), 31);
    }

    #[test]
    fn the_grid_can_start_on_sunday() {
        let grid = month_grid("2026-07-21", 0);
        assert_eq!(grid[0].iso, "2026-06-28", "the Sunday before");
    }

    #[test]
    fn the_month_label_follows_the_language() {
        assert_eq!(month_label("2026-07-21", "pt-br"), "Julho 2026");
        assert_eq!(month_label("2026-07-21", "en"), "July 2026");
        assert_eq!(month_label("2026-07-21", "ja"), "7月 2026");
    }

    #[test]
    fn the_weekday_initials_start_where_asked() {
        assert_eq!(weekday_labels("pt-br", 1), ["S", "T", "Q", "Q", "S", "S", "D"]);
        assert_eq!(weekday_labels("en", 0).len(), 7);
    }
}
