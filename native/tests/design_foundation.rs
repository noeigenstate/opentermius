//! Exercise shared controls with real pointer/key events on Slint's software backend.
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{
    Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, Model};
use std::rc::Rc;

slint::slint! {
    import { Theme, Action, Input, Toggle, Checkbox, Segmented, Select, EntityCard, ListRow, IconButton, Popover, MenuItem, Dialog } from "../ui/common.slint";
    export { Theme }
    export component FoundationHarness inherits Window {
        width: 600px; height: 700px;
        in-out property <int> actions: 0;
        in-out property <int> more-actions: 0;
        in-out property <int> copies: 0;
        in-out property <int> confirms: 0;
        in-out property <int> cancels: 0;
        in-out property <bool> checked <=> toggle.checked;
        in-out property <bool> boxed <=> checkbox.checked;
        in-out property <int> segment <=> segmented.current-index;
        in-out property <int> selection <=> select.current-index;
        in-out property <bool> select-open <=> select.open;
        in-out property <bool> menu-open <=> menu.open;
        in-out property <bool> dialog-open <=> dialog.open;
        in-out property <bool> confirm-enabled <=> dialog.confirm-enabled;
        out property <bool> disabled-focused: disabled.focused;
        out property <bool> dialog-input-focused: dialog-input.focused;
        out property <string> disabled-text: disabled.text;
        public function focus-disabled() { disabled.focus(); }
        public function focus-dialog-input() { dialog-input.focus(); }
        out property <[color]> palette: [Theme.base, Theme.lowest, Theme.high, Theme.highest,
            Theme.border, Theme.divider, Theme.secondary, Theme.muted, Theme.text, Theme.accent,
            Theme.accent-hover, Theme.accent-pressed, Theme.accent-soft, Theme.on-accent,
            Theme.danger, Theme.danger-hover, Theme.danger-soft, Theme.success, Theme.success-soft,
            Theme.warning, Theme.warning-soft, Theme.hover, Theme.pressed, Theme.selected,
            Theme.control, Theme.control-hover, Theme.field-bg, Theme.search-bg, Theme.focus-ring,
            Theme.scrim, Theme.tab-bg, Theme.tab-hover, Theme.tab-active, Theme.titlebar-fg,
            Theme.tile-host, Theme.folder, Theme.terminal-bg, Theme.terminal-fg,
            Theme.terminal-selection, Theme.terminal-cursor];
        Action { x: 10px; y: 10px; width: 100px; height: 30px; text: "Legacy"; primary: true; clicked => { root.actions += 1; } }
        Action { x: 120px; y: 10px; width: 100px; height: 30px; text: "Disabled"; disabled: true; clicked => { root.actions += 1; } }
        toggle := Toggle { x: 10px; y: 60px; }
        checkbox := Checkbox { x: 80px; y: 60px; width: 140px; text: "Include keys"; }
        segmented := Segmented { x: 10px; y: 100px; width: 220px; options: ["A", "B", "C"]; }
        disabled := Input { x: 10px; y: 150px; width: 180px; height: 36px; enabled: false; text: "Read only"; }
        select := Select { x: 10px; y: 200px; width: 180px; height: 36px; options: ["Local", "Remote", "Dynamic"]; }
        EntityCard { x: 10px; y: 280px; width: 300px; height: 60px; title: "Host";
            icon: @image-url("../ui/icons/hosts.svg");
            clicked => { root.actions += 1; } more-clicked => { root.more-actions += 1; }
        }
        ListRow { x: 10px; y: 350px; width: 300px; height: 40px; title: "Snippet";
            clicked => { root.actions += 1; }
            IconButton { icon: @image-url("../ui/icons/copy.svg"); clicked => { root.copies += 1; } }
        }
        menu := Popover { open: false; anchor-x: 350px; anchor-y: 200px;
            MenuItem { text: "Copy"; clicked => { root.copies += 1; } }
        }
        dialog := Dialog { title: "Confirm"; open: false;
            confirm => { root.confirms += 1; }
            cancel => { root.cancels += 1; }
            dialog-input := Input { placeholder-text: "Type here"; }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn click(ui: &FoundationHarness, x: f32, y: f32) {
    slint::platform::update_timers_and_animations();
    let position = slint::LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    slint::platform::update_timers_and_animations();
}

fn key(ui: &FoundationHarness, text: slint::SharedString) {
    slint::platform::update_timers_and_animations();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    slint::platform::update_timers_and_animations();
}

#[test]
fn palette_and_control_interactions() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    window.set_size(slint::PhysicalSize::new(600, 700));
    let ui = FoundationHarness::new().unwrap();
    ui.show().unwrap();

    // Compare every live palette token in both modes with the approved specification.
    let rows: Vec<_> = include_str!("../../docs/design-spec.md")
        .lines()
        .skip_while(|line| !line.starts_with("| `base` |"))
        .take_while(|line| line.starts_with('|'))
        .collect();
    assert_eq!(rows.len(), ui.get_palette().row_count());
    for dark in [false, true] {
        ui.global::<Theme>().set_dark(dark);
        for (index, row) in rows.iter().enumerate() {
            let columns: Vec<_> = row.split('|').map(str::trim).collect();
            let hex = columns[if dark { 3 } else { 2 }]
                .trim_matches('`')
                .trim_start_matches('#');
            let value = u32::from_str_radix(hex, 16).unwrap();
            let expected = if hex.len() == 8 {
                slint::Color::from_argb_u8(
                    value as u8,
                    (value >> 24) as u8,
                    (value >> 16) as u8,
                    (value >> 8) as u8,
                )
            } else {
                slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
            };
            assert_eq!(
                ui.get_palette().row_data(index).unwrap(),
                expected,
                "{} dark={dark}",
                columns[1]
            );
        }
    }
    click(&ui, 50., 25.);
    click(&ui, 170., 25.);
    assert_eq!(ui.get_actions(), 1, "disabled Action must not fire");
    click(&ui, 25., 70.);
    click(&ui, 100., 70.);
    assert!(ui.get_checked() && ui.get_boxed());
    click(&ui, 195., 115.);
    assert_eq!(ui.get_segment(), 2);
    ui.invoke_focus_disabled();
    key(&ui, "x".into());
    assert!(!ui.get_disabled_focused());
    assert_eq!(ui.get_disabled_text(), "Read only");

    click(&ui, 285., 310.);
    assert_eq!(ui.get_more_actions(), 1);
    assert_eq!(
        ui.get_actions(),
        1,
        "more button must not trigger card click"
    );
    click(&ui, 280., 370.);
    assert_eq!(ui.get_copies(), 1, "row must not cover its trailing button");
    assert_eq!(ui.get_actions(), 1);

    click(&ui, 50., 218.);
    assert!(ui.get_select_open());
    click(&ui, 50., 290.);
    assert_eq!(ui.get_selection(), 1);
    assert!(!ui.get_select_open());
    click(&ui, 50., 218.);
    key(&ui, Key::Escape.into());
    assert!(!ui.get_select_open(), "Escape closes Select");
    click(&ui, 50., 218.);
    click(&ui, 580., 680.);
    assert!(!ui.get_select_open(), "click-away closes Select");

    ui.set_menu_open(true);
    key(&ui, Key::Escape.into());
    assert!(!ui.get_menu_open());
    ui.set_menu_open(true);
    click(&ui, 580., 680.);
    assert!(!ui.get_menu_open());

    ui.set_dialog_open(true);
    slint::platform::update_timers_and_animations();
    ui.invoke_focus_dialog_input();
    assert!(ui.get_dialog_input_focused());
    key(&ui, Key::Return.into());
    assert_eq!(ui.get_confirms(), 1, "Enter in an Input confirms Dialog");
    ui.set_confirm_enabled(false);
    key(&ui, Key::Return.into());
    assert_eq!(ui.get_confirms(), 1);
    click(&ui, 580., 680.);
    assert!(ui.get_dialog_open(), "data-entry scrim must not dismiss");
    ui.invoke_focus_dialog_input();
    key(&ui, Key::Escape.into());
    assert!(!ui.get_dialog_open());
    assert_eq!(ui.get_cancels(), 1);
}
