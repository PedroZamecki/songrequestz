//! Native window: status, the request queue, commands and replies, settings and the log, in tabs.
//! Lives hidden in the tray; while hidden its native window is destroyed and nothing redraws.

use crate::commands::{Command, Commands, Replies, Who};
use crate::{server, status, Config, Player};
use fltk::browser::HoldBrowser;
use fltk::button::{Button, CheckButton};
use fltk::enums::{Align, ColorDepth, Event, Font, Shortcut};
use fltk::frame::Frame;
use fltk::group::{Flex, Scroll, ScrollType, Tabs};
use fltk::image::RgbImage;
use fltk::input::{Input, IntInput, MultilineInput, SecretInput};
use fltk::menu::{Choice, MenuFlag};
use fltk::misc::InputChoice;
use fltk::prelude::*;
use fltk::text::{TextBuffer, TextDisplay};
use fltk::window::Window;
use fltk::{app, dialog};
use serde_json::{json, Map, Value};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedSender, oneshot, watch};

pub enum Msg {
    /// Tray click: show or hide the window.
    Toggle,
    /// No tray icon (e.g. no StatusNotifier host on Linux): keep the window up, closing it quits.
    NoTray,
}

const ROW: i32 = 26;
const W: i32 = 820;
const H: i32 = 640;
const WHOS: [(Who, &str); 6] = [
    (Who::Everyone, "Everyone"),
    (Who::Followers, "Followers (TikTok)"),
    (Who::Subs, "Subscribers"),
    (Who::Vips, "VIPs"),
    (Who::Mods, "Moderators"),
    (Who::Broadcaster, "Broadcaster"),
];
const PAUSED: [&str; 3] = ["Keep the song", "Empty", "This text:"];

pub fn app() -> app::App {
    app::App::default().with_scheme(app::Scheme::Gtk)
}

/// Show a startup error; there's no console on Windows.
pub fn fatal(msg: &str) -> ! {
    eprintln!("{msg}");
    dialog::alert_default(msg);
    std::process::exit(1);
}

fn theme(dark: bool) {
    if dark {
        app::background(45, 45, 48);
        app::background2(30, 30, 32);
        app::foreground(225, 225, 225);
        app::set_selection_color(70, 110, 170);
    } else {
        app::background(192, 192, 192);
        app::background2(255, 255, 255);
        app::foreground(0, 0, 0);
        app::set_selection_color(0, 0, 128);
    }
}

/// A labelled input row.
fn field<I: InputExt + Default>(col: &mut Flex, label: &str, value: &str, tip: &str) -> I {
    let mut row = Flex::default().row();
    let l = Frame::default()
        .with_label(label)
        .with_align(Align::Left | Align::Inside);
    let mut input = I::default();
    input.set_value(value);
    if !tip.is_empty() {
        input.set_tooltip(tip);
    }
    row.end();
    row.fixed(&l, 170);
    col.fixed(&row, ROW);
    input
}

/// A labelled number.
fn number(col: &mut Flex, label: &str, value: impl ToString, tip: &str) -> IntInput {
    field(col, label, &value.to_string(), tip)
}

/// A line of text; `bold` for section titles.
fn text(col: &mut Flex, label: &str, bold: bool) -> Frame {
    let mut f = Frame::default()
        .with_label(label)
        .with_align(Align::Left | Align::Inside);
    if bold {
        f.set_label_font(Font::HelveticaBold);
    }
    col.fixed(&f, ROW - 4);
    f
}

fn check(col: &mut Flex, label: &str, on: bool) -> CheckButton {
    let c = CheckButton::default().with_label(label);
    c.set_checked(on);
    col.fixed(&c, ROW);
    c
}

fn button(row: &mut Flex, label: &str, w: i32) -> Button {
    let b = Button::default().with_label(label);
    row.fixed(&b, w);
    b
}

/// Replace a menu's items. Labels are set after adding: add() treats / \ _ as paths and flags,
/// and labels draw "&" and "@" specially unless doubled.
fn set_items(menu: &mut impl MenuExt, items: impl IntoIterator<Item = String>) {
    menu.clear();
    for (i, label) in items.into_iter().enumerate() {
        menu.add("x", Shortcut::None, MenuFlag::Normal, |_| {});
        if let Some(mut item) = menu.at(i as i32) {
            item.set_label(&label.replace('&', "&&").replace('@', "@@"));
        }
    }
}

/// A Streamer.bot action: type it, or pick it from Streamer.bot's list.
fn action(value: &str) -> InputChoice {
    let mut a = InputChoice::default();
    a.set_value(value);
    a.set_tooltip("Streamer.bot action (empty: none). The list fills in once Streamer.bot is connected.");
    a
}

/// Offer Streamer.bot's actions in a picker; picking one copies its name into the field.
fn offer(a: &mut InputChoice, names: &[String]) {
    let keep = a.value().unwrap_or_default();
    let mut menu = a.menu_button();
    set_items(&mut menu, std::iter::once(String::new()).chain(names.iter().cloned()));
    let names: Vec<String> = std::iter::once(String::new()).chain(names.iter().cloned()).collect();
    let mut input = a.input();
    menu.set_callback(move |m| {
        if let Some(n) = names.get(m.value().max(0) as usize) {
            input.set_value(n);
        }
    });
    a.set_value(&keep);
}

fn lines(v: &[String]) -> String {
    v.join("\n")
}

fn unlines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn tab(tabs: &Tabs, label: &str) -> Flex {
    let mut col = Flex::new(tabs.x(), tabs.y() + 25, tabs.w(), tabs.h() - 25, None)
        .column()
        .with_label(label);
    col.set_margin(10);
    col.set_pad(4);
    col
}

/// A tab too long for the window: a scrolling column, `rows` rows high.
fn long_tab(tabs: &Tabs, label: &str, height: i32) -> (Scroll, Flex) {
    let mut s = Scroll::new(tabs.x(), tabs.y() + 25, tabs.w(), tabs.h() - 25, None).with_label(label);
    s.set_type(ScrollType::Vertical);
    let mut col = Flex::new(s.x(), s.y(), s.w() - 20, height, None).column();
    col.set_margin(10);
    col.set_pad(4);
    (s, col)
}

/// Tell the app to run a Songify API action (its answer goes to the log).
fn act(api: &UnboundedSender<server::Command>, action: &str, data: Value) {
    let _ = api.send((action.into(), data, oneshot::channel().0));
}

struct CmdRow {
    name: &'static str,
    on: CheckButton,
    trigger: Input,
    who: Choice,
    action: InputChoice,
    reply: Input,
}

pub fn run(
    _app: app::App,
    title: &str,
    cfg: Arc<watch::Sender<Config>>,
    rx: app::Receiver<Msg>,
    api: UnboundedSender<server::Command>,
    show: bool,
) {
    let c = cfg.borrow().clone();
    let r = &c.requests;
    theme(c.dark);
    let mut win = Window::default().with_size(W, H).with_label(title);
    win.set_xclass("songrequestz");
    use crate::tray::{ICON, SIZE};
    if let Ok(icon) = RgbImage::new(ICON, SIZE, SIZE, ColorDepth::Rgba8) {
        win.set_icon(Some(icon));
    }
    let tabs = Tabs::new(5, 5, W - 10, H - 50, None);

    // Status
    let mut col = tab(&tabs, "Status");
    let info = Frame::default().with_align(Align::Left | Align::Inside | Align::Top);
    col.fixed(&info, 110);
    let mut open = check(&mut col, "Song requests open", r.open);
    Frame::default();
    text(
        &mut col,
        &format!(
            "Overlays: http://127.0.0.1:{0}/ and ws://127.0.0.1:{0}/ws/data (Songify widgets: add ?ws={0})",
            c.port
        ),
        false,
    );
    text(
        &mut col,
        &format!("Settings file: {}", crate::config_path().display()),
        false,
    );
    col.end();

    // Queue
    let mut col = tab(&tabs, "Queue");
    let now = text(&mut col, "", true);
    let queue = HoldBrowser::default();
    let mut bar = Flex::default().row();
    let mut play = button(&mut bar, "Play/Pause", 110);
    let mut skip = button(&mut bar, "Skip", 90);
    let mut remove = button(&mut bar, "Remove request", 140);
    Frame::default();
    bar.end();
    col.fixed(&bar, ROW + 4);
    col.end();

    // Commands and replies
    let replies = serde_json::to_value(&r.replies).unwrap_or_default();
    let replies = replies.as_object().cloned().unwrap_or_default();
    let all = r.commands.all();
    let height = (all.len() + replies.len() + 4) as i32 * (ROW + 4) + 20;
    let (scroll, mut col) = long_tab(&tabs, "Commands", height);
    text(
        &mut col,
        "On, command (without !), who may use it, its Streamer.bot action (else the default one) and reply:",
        false,
    );
    let mut cmds: Vec<CmdRow> = Vec::new();
    for (name, cmd) in all {
        let mut row = Flex::default().row();
        let on = CheckButton::default();
        on.set_checked(cmd.enabled);
        let mut trigger = Input::default();
        trigger.set_value(&cmd.trigger);
        let mut who = Choice::default();
        set_items(&mut who, WHOS.iter().map(|(_, l)| l.to_string()));
        who.set_value(WHOS.iter().position(|(w, _)| *w == cmd.who).unwrap_or(0) as i32);
        let action = action(&cmd.action);
        let mut reply = Input::default();
        reply.set_value(&cmd.reply);
        row.end();
        row.fixed(&on, 24);
        row.fixed(&trigger, 90);
        row.fixed(&who, 150);
        row.fixed(&action, 170);
        col.fixed(&row, ROW);
        cmds.push(CmdRow {
            name,
            on,
            trigger,
            who,
            action,
            reply,
        });
    }
    text(&mut col, "", false);
    text(
        &mut col,
        "Other replies (empty: nothing in chat; the action still runs)",
        true,
    );
    let reply_inputs: Vec<(String, Input)> = replies
        .iter()
        .map(|(k, v)| {
            let label = k.replace('_', " ");
            let i: Input = field(&mut col, &label, v.as_str().unwrap_or(""), "");
            (k.clone(), i)
        })
        .collect();
    col.end();
    scroll.end();

    // Settings
    let (scroll, mut col) = long_tab(&tabs, "Settings", 33 * (ROW + 4) + 3 * 3 * ROW);
    text(&mut col, "Player", true);
    let player = {
        let mut row = Flex::default().row();
        let l = Frame::default()
            .with_label("Music player")
            .with_align(Align::Left | Align::Inside);
        let mut ch = Choice::default();
        set_items(
            &mut ch,
            ["Pear Desktop", "Spotify app (no song requests)"].map(String::from),
        );
        ch.set_value((c.player == Player::Spotify) as i32);
        ch.set_tooltip("Spotify: now playing, play/pause and skip from the desktop app, without its Web API (Premium)");
        row.end();
        row.fixed(&l, 170);
        row.fixed(&ch, 260);
        col.fixed(&row, ROW);
        ch
    };
    text(&mut col, "Streamer.bot", true);
    let sb_url: Input = field(&mut col, "WebSocket URL", &c.streamerbot_url, "Empty: off");
    let sb_password: SecretInput = field(
        &mut col,
        "Password",
        &c.streamerbot_password,
        "When its WebSocket authentication is on (needed for replies straight to chat)",
    );
    let reply_action = {
        let mut row = Flex::default().row();
        let l = Frame::default()
            .with_label("Default reply action")
            .with_align(Align::Left | Align::Inside);
        let a = action(&c.reply_action);
        row.end();
        row.fixed(&l, 170);
        col.fixed(&row, ROW);
        a
    };
    text(&mut col, "TikTok (through tikstream or TikFinity)", true);
    let tt_url: Input = field(&mut col, "WebSocket URL", &c.tiktok_url, "Empty: off");
    let tt_user: Input = field(
        &mut col,
        "Your TikTok @@name",
        &c.tiktok_user,
        "Counts as the broadcaster",
    );
    let tt_action = {
        let mut row = Flex::default().row();
        let l = Frame::default()
            .with_label("Result action")
            .with_align(Align::Left | Align::Inside);
        let a = action(&c.tiktok_action);
        row.end();
        row.fixed(&l, 170);
        col.fixed(&row, ROW);
        a
    };
    text(&mut col, "Requests (0: no limit)", true);
    let max_queue = number(&mut col, "Queue length", r.max_queue, "");
    let max_per_user = number(&mut col, "Per user", r.max_per_user, "");
    let max_minutes = number(&mut col, "Song length (minutes)", r.max_minutes, "");
    let cooldown = number(
        &mut col,
        "Cooldown (seconds)",
        r.cooldown_s,
        "Between one user's requests",
    );
    let votes = number(
        &mut col,
        "Votes to skip",
        r.votes_needed,
        "Distinct voters for !voteskip",
    );
    let mut block = |label: &str, v: &[String]| {
        let mut row = Flex::default().row();
        let l = Frame::default()
            .with_label(label)
            .with_align(Align::Left | Align::Inside | Align::Top);
        let mut i = MultilineInput::default();
        i.set_value(&lines(v));
        i.set_tooltip("One per line, any case");
        row.end();
        row.fixed(&l, 170);
        col.fixed(&row, 3 * ROW);
        i
    };
    let blocked_users = block("Blocked users", &r.blocked_users);
    let blocked_artists = block("Blocked artists", &r.blocked_artists);
    let blocked_songs = block("Blocked songs (id or title)", &r.blocked_songs);
    text(&mut col, "Songify API and files", true);
    let port = number(&mut col, "Port (needs a restart)", c.port, "Songify's is 65530");
    let api_password: SecretInput = field(&mut col, "Password", &c.api_password, "Empty: none");
    let files_dir: Input = field(
        &mut col,
        "Files folder",
        &c.files_dir,
        "Songify.txt and cover.png; empty: next to the exe",
    );
    let output: Input = field(
        &mut col,
        "Songify.txt",
        &c.output,
        "{artist} {single_artist} {title} {req} {url} {uri}; {{...}} only when its placeholders aren't empty",
    );
    let (mut paused, paused_text) = {
        let mut row = Flex::default().row();
        let l = Frame::default()
            .with_label("While paused")
            .with_align(Align::Left | Align::Inside);
        let mut ch = Choice::default();
        set_items(&mut ch, PAUSED.iter().map(|s| s.to_string()));
        let mut t = Input::default();
        let (i, v) = match &c.paused_text {
            None => (0, ""),
            Some(t) if t.is_empty() => (1, ""),
            Some(t) => (2, t.as_str()),
        };
        ch.set_value(i);
        t.set_value(v);
        row.end();
        row.fixed(&l, 170);
        row.fixed(&ch, 140);
        col.fixed(&row, ROW);
        (ch, t)
    };
    text(&mut col, "Window", true);
    let dark = check(&mut col, "Dark theme", c.dark);
    let hidden = check(&mut col, "Start hidden in the tray", c.start_hidden);
    let autostart = check(
        &mut col,
        if cfg!(windows) {
            "Start with Windows"
        } else {
            "Start when I log in"
        },
        c.autostart,
    );
    col.end();
    scroll.end();

    // Log
    let col = tab(&tabs, "Log");
    let mut log = TextDisplay::default();
    let buf = TextBuffer::default();
    log.set_buffer(buf.clone());
    col.end();
    tabs.end();

    let mut bar = Flex::new(10, H - 40, W - 20, 32, None).row();
    let mut save = button(&mut bar, "Save", 90);
    save.set_shortcut(Shortcut::Ctrl | 's');
    save.set_tooltip("Save and apply (Ctrl+S)");
    let mut saved = Frame::default().with_align(Align::Left | Align::Inside);
    let mut hide = button(&mut bar, "Hide", 90);
    let mut quit = button(&mut bar, "Quit", 90);
    bar.end();
    win.end();
    win.resizable(&tabs);
    paused.set_callback({
        let mut t = paused_text.clone();
        move |c| if c.value() == 2 { t.activate() } else { t.deactivate() }
    });
    paused.do_callback();
    dark.clone().set_callback({
        let win = win.clone();
        move |b| {
            theme(b.is_checked());
            win.clone().redraw();
        }
    });

    open.set_callback({
        let cfg = cfg.clone();
        move |b| {
            cfg.send_modify(|c| c.requests.open = b.is_checked());
            let _ = crate::save(&cfg.borrow());
        }
    });
    play.set_callback({
        let api = api.clone();
        move |_| act(&api, "play_pause", Value::Null)
    });
    skip.set_callback({
        let api = api.clone();
        move |_| act(&api, "skip", Value::Null)
    });
    remove.set_callback({
        let (api, queue) = (api.clone(), queue.clone());
        move |_| {
            if queue.value() > 0 {
                act(&api, "queue_remove", json!({ "index": queue.value() - 1 }));
            }
        }
    });

    // What the once-a-second refresh updates besides the status: action pickers and the settings
    // chat can change.
    let mut pickers = vec![reply_action.clone(), tt_action.clone()];
    pickers.extend(cmds.iter().map(|r| r.action.clone()));
    let blocked = [blocked_users.clone(), blocked_artists.clone(), blocked_songs.clone()];
    save.set_callback({
        let cfg = cfg.clone();
        move |_| {
            let int = |i: &IntInput| i.value().trim().parse::<u64>().unwrap_or(0);
            let mut commands = Map::new();
            for r in &cmds {
                let cmd = Command {
                    trigger: r.trigger.value().trim().trim_start_matches('!').into(),
                    enabled: r.on.is_checked(),
                    who: WHOS[r.who.value().clamp(0, 5) as usize].0,
                    reply: r.reply.value(),
                    action: r.action.value().unwrap_or_default().trim().into(),
                };
                commands.insert(r.name.into(), serde_json::to_value(cmd).unwrap_or_default());
            }
            let replies: Map<String, Value> = reply_inputs
                .iter()
                .map(|(k, i)| (k.clone(), json!(i.value())))
                .collect();
            cfg.send_modify(|c| {
                c.player = if player.value() == 1 {
                    Player::Spotify
                } else {
                    Player::Pear
                };
                c.streamerbot_url = sb_url.value().trim().into();
                c.streamerbot_password = sb_password.value();
                c.reply_action = reply_action.value().unwrap_or_default().trim().into();
                c.tiktok_url = tt_url.value().trim().into();
                c.tiktok_user = tt_user.value().trim().trim_start_matches('@').into();
                c.tiktok_action = tt_action.value().unwrap_or_default().trim().into();
                c.port = int(&port).clamp(1, 65535) as u16;
                c.api_password = api_password.value();
                c.files_dir = files_dir.value().trim().into();
                c.output = output.value();
                c.paused_text = match paused.value() {
                    0 => None,
                    1 => Some(String::new()),
                    _ => Some(paused_text.value()),
                };
                c.dark = dark.is_checked();
                c.start_hidden = hidden.is_checked();
                c.autostart = autostart.is_checked();
                let r = &mut c.requests;
                r.max_queue = int(&max_queue) as usize;
                r.max_per_user = int(&max_per_user) as usize;
                r.max_minutes = int(&max_minutes) as u32;
                r.cooldown_s = int(&cooldown);
                r.votes_needed = int(&votes).max(1) as usize;
                r.blocked_users = unlines(&blocked_users.value());
                r.blocked_artists = unlines(&blocked_artists.value());
                r.blocked_songs = unlines(&blocked_songs.value());
                if let Ok(cmds) = serde_json::from_value::<Commands>(Value::Object(commands)) {
                    r.commands = cmds;
                }
                if let Ok(replies) = serde_json::from_value::<Replies>(Value::Object(replies)) {
                    r.replies = replies;
                }
            });
            let c = cfg.borrow().clone();
            let done = crate::save(&c).and_then(|()| crate::autostart(c.autostart));
            saved.set_label(&match done {
                Ok(()) => "Saved, applied.".to_string(),
                Err(e) => format!("Can't save: {e}"),
            });
        }
    });
    quit.set_callback(|_| std::process::exit(0));
    hide.set_callback({
        let mut win = win.clone();
        move |_| win.hide()
    });
    let tray_ok = Rc::new(Cell::new(true));
    win.set_callback({
        let tray_ok = tray_ok.clone();
        move |w| {
            if app::event() == Event::Close {
                if tray_ok.get() {
                    w.hide();
                } else {
                    std::process::exit(0);
                }
            }
        }
    });

    // Redraw once a second, only while the window is up, and only what changed.
    let ticking = Rc::new(Cell::new(false));
    let start = {
        let (win, config) = (win.clone(), cfg.subscribe());
        move || {
            if ticking.replace(true) {
                return;
            }
            let (win, mut info, mut now, mut queue, mut log, mut buf, ticking) = (
                win.clone(),
                info.clone(),
                now.clone(),
                queue.clone(),
                log.clone(),
                buf.clone(),
                ticking.clone(),
            );
            let (open, mut blocked, mut pickers, mut config) =
                (open.clone(), blocked.clone(), pickers.clone(), config.clone());
            config.mark_changed();
            let (mut seen_log, mut seen_queue, mut seen_actions, mut shown) = (u64::MAX, u64::MAX, u64::MAX, 0usize);
            app::add_timeout3(0.0, move |h| {
                if !win.visible() {
                    ticking.set(false);
                    return;
                }
                let (mut text, mut full, mut append, mut list, mut actions) = (String::new(), None, None, None, None);
                status(|s| {
                    let or = |v: &str| if v.is_empty() { "starting..." } else { v }.to_string();
                    let playing = if s.now.is_empty() { "nothing" } else { &s.now };
                    text = format!(
                        "Now playing: {playing}\nRequests: {}\nStreamer.bot: {}\nTikTok: {}\nPlayer: {}",
                        s.queue.len(),
                        or(&s.sb),
                        or(&s.tiktok),
                        or(&s.player)
                    );
                    if s.queue_rev != seen_queue {
                        seen_queue = s.queue_rev;
                        list = Some((s.now.clone(), s.queue.clone()));
                    }
                    if s.actions_rev != seen_actions {
                        seen_actions = s.actions_rev;
                        actions = Some(s.actions.clone());
                    }
                    if s.logged != seen_log {
                        // Append only the new lines; rebuild after a clear or when far behind.
                        let new = s.logged.wrapping_sub(seen_log) as usize;
                        if seen_log == u64::MAX || new > s.log.len() || shown + new > 2 * crate::LOG_LINES {
                            full = Some(s.log.iter().map(String::as_str).collect::<Vec<_>>().join("\n"));
                            shown = s.log.len();
                        } else {
                            let skip = s.log.len() - new;
                            append = Some(s.log.iter().skip(skip).fold(String::new(), |a, l| a + "\n" + l));
                            shown += new;
                        }
                        seen_log = s.logged;
                    }
                });
                // FLTK labels treat "@" as a symbol marker; "@@" is a literal "@".
                let text = text.replace('@', "@@");
                if info.label() != text {
                    info.set_label(&text);
                }
                if let Some((playing, items)) = list {
                    let playing = if playing.is_empty() {
                        "Nothing playing".into()
                    } else {
                        playing
                    };
                    now.set_label(&playing.replace('@', "@@"));
                    let keep = queue.value();
                    queue.clear();
                    for i in &items {
                        // Lines start with "#", so a "@" in them is never a format code.
                        queue.add(i);
                    }
                    queue.select(keep.min(items.len() as i32));
                }
                if let Some(names) = actions {
                    for p in pickers.iter_mut() {
                        offer(p, &names);
                    }
                }
                // Changed by chat or the API (!togglesr, !bansong, block_*): show it.
                if config.has_changed().unwrap_or(false) {
                    let c = config.borrow_and_update();
                    let r = &c.requests;
                    open.set_checked(r.open);
                    for (i, v) in blocked
                        .iter_mut()
                        .zip([&r.blocked_users, &r.blocked_artists, &r.blocked_songs])
                    {
                        if unlines(&i.value()) != *v {
                            i.set_value(&lines(v));
                        }
                    }
                }
                let scroll = full.is_some() || append.is_some();
                if let Some(full) = full {
                    buf.set_text(&full);
                }
                if let Some(append) = append {
                    // The first line has no newline before it.
                    buf.append(if buf.length() == 0 { &append[1..] } else { &append });
                }
                if scroll {
                    log.scroll(log.count_lines(0, buf.length(), true), 0);
                }
                app::repeat_timeout3(1.0, h);
            });
        }
    };

    if show {
        win.show();
        start();
    }
    // Not app.run() or app.wait(): with no window shown they return at once (busy loop / exit).
    // wait_for blocks until a tray message or UI event arrives.
    loop {
        let _ = app::wait_for(1e9);
        while let Some(m) = rx.recv() {
            match m {
                Msg::Toggle if win.visible() => {
                    win.hide();
                    continue;
                }
                Msg::Toggle => {}
                Msg::NoTray => {
                    tray_ok.set(false);
                    hide.deactivate();
                }
            }
            if !win.visible() {
                win.show();
                start();
            }
        }
    }
}
