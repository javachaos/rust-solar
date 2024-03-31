use tui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols,
    text::{Span, Spans},
    widgets::{Block, Borders},
};

#[derive(Debug, Clone)]
/// A custom widget for a toggle switch.
pub(crate) struct LoadToggleSwitch<'a> {
    pub(crate) is_on: bool,
    labels: (&'a str, &'a str),
}

impl<'a> LoadToggleSwitch<'a> {
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
