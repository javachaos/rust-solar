//! Small custom TUI widget used to render the controller's load output state.

use tui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols,
    text::{Span, Spans},
    widgets::{Block, Borders},
};

/// A custom widget for a toggle switch.
#[derive(Debug, Clone)]
pub(crate) struct LoadToggleSwitch<'a> {
    pub(crate) is_on: bool,
    labels: (&'a str, &'a str),
}

impl<'a> LoadToggleSwitch<'a> {
    /// Creates a load toggle widget with `(on, off)` labels.
    pub fn new(is_on: bool, labels: (&'a str, &'a str)) -> LoadToggleSwitch<'a> {
        LoadToggleSwitch { is_on, labels }
    }
}

impl<'a> tui::widgets::Widget for LoadToggleSwitch<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let on_label = Span::styled(
            self.labels.0,
            Style::default().fg(if self.is_on {
                Color::Green
            } else {
                Color::DarkGray
            }),
        );
        let off_label = Span::styled(
            self.labels.1,
            Style::default().fg(if !self.is_on {
                Color::Red
            } else {
                Color::DarkGray
            }),
        );

        let switch = if self.is_on {
            Span::styled(
                symbols::line::VERTICAL,
                Style::default().add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(" ")
        };

        let spans = Spans::from(vec![on_label, switch, off_label]);
        let block = Block::default().borders(Borders::ALL).title("Load");
        let inner_area = block.inner(area);
        block.render(area, buf);
        buf.set_spans(inner_area.x, inner_area.y, &spans, inner_area.width);
    }
}

#[cfg(test)]
mod tests {
    use tui::{
        buffer::Buffer,
        layout::Rect,
        style::{Color, Modifier},
        widgets::Widget,
    };

    use super::LoadToggleSwitch;

    #[test]
    fn renders_on_state_with_green_label() {
        let area = Rect::new(0, 0, 10, 3);
        let mut buffer = Buffer::empty(area);

        LoadToggleSwitch::new(true, ("ON", "OFF")).render(area, &mut buffer);

        assert_eq!(buffer.get(1, 1).symbol, "O");
        assert_eq!(buffer.get(2, 1).symbol, "N");
        assert_eq!(buffer.get(1, 1).fg, Color::Green);
        assert!(buffer.get(3, 1).modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn renders_off_state_with_red_label() {
        let area = Rect::new(0, 0, 10, 3);
        let mut buffer = Buffer::empty(area);

        LoadToggleSwitch::new(false, ("ON", "OFF")).render(area, &mut buffer);

        assert_eq!(buffer.get(4, 1).symbol, "O");
        assert_eq!(buffer.get(5, 1).symbol, "F");
        assert_eq!(buffer.get(6, 1).symbol, "F");
        assert_eq!(buffer.get(4, 1).fg, Color::Red);
        assert_eq!(buffer.get(3, 1).symbol, " ");
    }
}
