//! Shared widget helpers.

use iced::widget::{Space, button, center, column, container, opaque, row, stack, text};
use iced::{Border, Color, Element, Fill, Font, Theme};

use super::Message;

// Layout tokens: every view takes sizes from here so edges and rows line up (design pass
// 2026-11-24, spec in the project plans folder).
/// Base grid unit; all paddings and gaps are multiples of it.
pub(crate) const U: f32 = 4.0;
/// Gap between controls in a row and between rows in a card.
pub(crate) const GAP: f32 = 2.0 * U;
/// Gap between sections / cards.
pub(crate) const SECTION_GAP: f32 = 4.0 * U;
/// Inner padding of cards, dialogs' content and the window edge.
pub(crate) const PAD: f32 = 3.0 * U;
/// Height of every single-line control: buttons, inputs, pick lists, list rows' text block.
pub(crate) const CONTROL_H: f32 = 32.0;
/// Width of the label column in every form (editor header, Common, properties, dialogs).
pub(crate) const LABEL_W: f32 = 128.0;
/// Width of trailing helper controls in a form row (Pick… lists, "Copy from window…").
pub(crate) const SIDE_W: f32 = 168.0;
/// Width of the rule / route list column.
pub(crate) const LIST_W: f32 = 340.0;
/// Fixed height of a list row (title + one-line subtitle).
pub(crate) const ROW_H: f32 = 48.0;
/// Max width of a form column; the form is centred in the space it gets.
pub(crate) const FORM_MAX_W: f32 = 960.0;
/// Type scale: body text and labels, section headings, secondary/help text, dialog titles.
pub(crate) const TEXT: f32 = 14.0;
pub(crate) const TEXT_HEAD: f32 = 15.0;
pub(crate) const TEXT_SMALL: f32 = 12.0;
pub(crate) const TEXT_TITLE: f32 = 18.0;
/// Corner radius for buttons, inputs, cards, badges.
pub(crate) const RADIUS: f32 = 6.0;

pub(super) const ITALIC: Font = Font {
    style: iced::font::Style::Italic,
    ..Font::DEFAULT
};

pub(super) fn overlay<'a>(
    base: Element<'a, Message>,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let dialog = container(content)
        .padding(2.0 * PAD)
        .style(container::bordered_box);
    stack![
        base,
        opaque(center(opaque(dialog)).style(|_| {
            container::Style {
                background: Some(
                    Color {
                        a: 0.55,
                        ..Color::BLACK
                    }
                    .into(),
                ),
                ..Default::default()
            }
        })),
    ]
    .into()
}

/// Toolbar and row action buttons: visible at rest, with a distinct disabled look.
pub(crate) fn action(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
    let base = button::Style {
        background: Some(p.background.weak.color.into()),
        text_color: p.background.base.text,
        border: Border {
            color: p.background.strong.color,
            width: 1.0,
            radius: RADIUS.into(),
        },
        ..Default::default()
    };
    match status {
        button::Status::Active => base,
        button::Status::Hovered => button::Style {
            background: Some(p.primary.weak.color.into()),
            text_color: p.primary.weak.text,
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(p.primary.base.color.into()),
            text_color: p.primary.base.text,
            ..base
        },
        button::Status::Disabled => button::Style {
            background: None,
            text_color: p.background.base.text.scale_alpha(0.35),
            border: Border {
                color: p.background.weak.color,
                ..base.border
            },
            ..base
        },
    }
}

/// Tab button: flat, primary text when active, secondary text otherwise. The underline is
/// drawn by the caller so it is exactly as wide as the tab.
pub(super) fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.extended_palette();
        let text_color = match (active, status) {
            (true, _) => p.background.base.text,
            (false, button::Status::Hovered | button::Status::Pressed) => p.background.base.text,
            (false, _) => p.background.base.text.scale_alpha(0.6),
        };
        button::Style {
            text_color,
            border: Border {
                radius: RADIUS.into(),
                ..Border::default()
            },
            ..Default::default()
        }
    }
}

/// Vertically centres button content (iced's button leaves it at the top).
pub(crate) fn centered<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
) -> container::Container<'a, M> {
    container(content).center_y(Fill)
}

/// Standard control button: label on one line, height CONTROL_H, style `action`.
pub(crate) fn tool_button<'a, M: Clone + 'a>(
    label: impl text::IntoFragment<'a>,
) -> button::Button<'a, M> {
    button(centered(single_line(label, TEXT)))
        .height(CONTROL_H)
        .padding([0.0, 12.0])
        .style(action)
}

/// Single-line text input of height CONTROL_H (line height carries the height; no vertical padding).
pub(crate) fn input<'a, M: Clone + 'a>(
    placeholder: &str,
    value: &str,
) -> iced::widget::TextInput<'a, M> {
    iced::widget::text_input(placeholder, value)
        .size(TEXT)
        .padding([0.0, 12.0])
        .line_height(iced::widget::text::LineHeight::Absolute(CONTROL_H.into()))
}

/// Subtitle on the selected (primary-coloured) list row.
pub(crate) fn selected_subtitle(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().primary.base.text.scale_alpha(0.85)),
    }
}

fn pill(background: iced::Color, text_color: iced::Color) -> container::Style {
    container::Style {
        background: Some(background.into()),
        text_color: Some(text_color),
        border: Border {
            radius: 10.0.into(),
            ..Border::default()
        },
        ..Default::default()
    }
}

/// Match-count pill.
pub(crate) fn badge(theme: &Theme) -> container::Style {
    let p = &theme.extended_palette().primary.weak;
    pill(p.color, p.text)
}

/// Warning pill.
pub(crate) fn warn_badge(theme: &Theme) -> container::Style {
    let p = &theme.extended_palette().danger.weak;
    pill(p.color, p.text)
}

/// Thin primary-coloured line (tab underline, drop marker).
pub(super) fn rule_line(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().primary.base.color.into()),
        ..Default::default()
    }
}

/// Section heading + bordered card, sized by the layout tokens.
pub(crate) fn section<'a, M: 'a>(
    title: impl text::IntoFragment<'a>,
    content: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    column![
        text(title).size(TEXT_HEAD).font(BOLD),
        container(content)
            .padding(PAD)
            .width(Fill)
            .style(container::bordered_box),
    ]
    .spacing(GAP)
    .into()
}

/// One form row: a LABEL_W label column, vertically centred, then the control.
pub(crate) fn form_row<'a, M: 'a>(
    label: impl text::IntoFragment<'a>,
    control: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    row![
        container(single_line(label, TEXT)).width(LABEL_W),
        control.into(),
    ]
    .spacing(GAP)
    .align_y(iced::Center)
    .into()
}

/// Text that never wraps; the caller's container clips it.
pub(crate) fn single_line<'a>(content: impl text::IntoFragment<'a>, size: f32) -> text::Text<'a> {
    text(content)
        .size(size)
        .wrapping(iced::widget::text::Wrapping::None)
}

/// Pill badge: fixed height 20, min width 24, centred TEXT_SMALL. `style` is badge or warn_badge.
pub(crate) fn pill_badge<'a, M: 'a>(
    label: impl text::IntoFragment<'a>,
    style: fn(&Theme) -> container::Style,
) -> Element<'a, M> {
    container(text(label).size(TEXT_SMALL))
        .height(20)
        .padding([0, 7])
        .center_y(20)
        .align_x(iced::Center)
        .style(style)
        .into()
}

/// 1 px separators in the palette's strong background colour.
pub(crate) fn divider_h<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new())
        .height(1)
        .width(Fill)
        .style(divider)
        .into()
}

pub(crate) fn divider_v<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new())
        .width(1)
        .height(CONTROL_H)
        .style(divider)
        .into()
}

fn divider(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.strong.color.into()),
        ..Default::default()
    }
}

pub(crate) const BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..Font::DEFAULT
};

pub(crate) const MONO: Font = Font::MONOSPACE;
