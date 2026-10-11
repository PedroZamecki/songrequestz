//! Tray icon: click (or "Show/Hide") toggles the window, "Quit" exits; the tooltip shows the song
//! and the connections. Without a tray (e.g. no StatusNotifier host on
//! Linux) the window stays up instead.

use crate::summary;
use crate::ui::Msg;
use fltk::app::Sender;

// ponytail: pre-shrunk raw pixels, no image decoder. After changing the icon run:
// magick icon_songrequestz.png -resize 64x64 -depth 8 rgba:src/icon.rgba
pub const ICON: &[u8] = include_bytes!("icon.rgba");
pub const SIZE: i32 = 64;

/// How often the tooltip is refreshed (only sent to the tray when it changed).
const TIP_SECS: u64 = 5;

#[cfg(target_os = "linux")]
pub async fn run(ui: Sender<Msg>) {
    use ksni::TrayMethods;

    struct Tray(Sender<Msg>, String);
    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "songrequestz".into()
        }
        fn title(&self) -> String {
            "songrequestz".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            // RGBA to ARGB, as StatusNotifierItem wants.
            let data = ICON
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[r, g, b, a]| [a, r, g, b])
                .collect();
            vec![ksni::Icon {
                width: SIZE,
                height: SIZE,
                data,
            }]
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            let (title, description) = self.1.split_once('\n').unwrap_or((&self.1, ""));
            ksni::ToolTip {
                title: title.into(),
                description: description.into(),
                ..Default::default()
            }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            self.0.send(Msg::Toggle);
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            vec![
                StandardItem {
                    label: "Show/Hide".into(),
                    activate: Box::new(|t: &mut Self| t.0.send(Msg::Toggle)),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Quit".into(),
                    activate: Box::new(|_| std::process::exit(0)),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    match Tray(ui, summary()).spawn().await {
        Ok(handle) => loop {
            tokio::time::sleep(std::time::Duration::from_secs(TIP_SECS)).await;
            let tip = summary();
            handle.update(|t| (t.1 != tip).then(|| t.1 = tip)).await;
        },
        Err(e) => {
            crate::say(format!("no tray icon ({e})"));
            ui.send(Msg::NoTray);
        }
    }
}

#[cfg(windows)]
pub async fn run(ui: Sender<Msg>) {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, SetTimer, TranslateMessage, MSG, WM_TIMER,
    };

    // Win32 delivers tray clicks to the thread that created the icon, so it gets its own
    // thread with a message loop.
    std::thread::spawn(move || {
        let open = MenuItem::new("Show/Hide", true, None);
        let close = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        let fail = |e: String| {
            crate::say(format!("no tray icon ({e})"));
            ui.send(Msg::NoTray);
        };
        if let Err(e) = menu.append_items(&[&open, &close]) {
            return fail(e.to_string());
        }
        let (open_id, close_id) = (open.id().clone(), close.id().clone());
        let menu_ui = ui;
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if e.id == open_id {
                menu_ui.send(Msg::Toggle);
            } else if e.id == close_id {
                std::process::exit(0);
            }
        }));
        // Left click toggles the window; right click opens the menu.
        let click_ui = ui;
        TrayIconEvent::set_event_handler(Some(move |e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                click_ui.send(Msg::Toggle);
            }
        }));

        let mut tip = summary();
        let tray = Icon::from_rgba(ICON.to_vec(), SIZE as u32, SIZE as u32)
            .map_err(|e| e.to_string())
            .and_then(|icon| {
                TrayIconBuilder::new()
                    .with_icon(icon)
                    .with_tooltip(win_tip(&tip))
                    .with_menu(Box::new(menu))
                    .with_menu_on_left_click(false)
                    .build()
                    .map_err(|e| e.to_string())
            });
        let tray = match tray {
            Ok(t) => t,
            Err(e) => return fail(e),
        };

        // SAFETY: standard Win32 message loop; MSG is plain data, zeroed is a valid initial value.
        // The thread timer posts WM_TIMER here every TIP_SECS for the tooltip.
        unsafe {
            SetTimer(std::ptr::null_mut(), 0, (TIP_SECS * 1000) as u32, None);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_TIMER {
                    let now = summary();
                    if now != tip {
                        let _ = tray.set_tooltip(Some(win_tip(&now)));
                        tip = now;
                    }
                    continue;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
}

/// Windows tooltips are one line of at most 127 characters.
#[cfg(windows)]
fn win_tip(s: &str) -> String {
    s.replace('\n', " | ").chars().take(127).collect()
}

#[cfg(not(any(target_os = "linux", windows)))]
pub async fn run(ui: Sender<Msg>) {
    ui.send(Msg::NoTray);
}
