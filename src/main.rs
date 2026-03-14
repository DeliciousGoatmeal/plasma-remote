use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, MouseButton, MouseEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect, Alignment},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, BorderType, Paragraph},
    Terminal,
};
use std::{io::{self, stdout}, process::Command, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

#[derive(Clone, Copy, PartialEq)]
enum TileMode {
    LeftHalf, RightHalf, TopLeft, TopRight, BotLeft, BotRight
}

#[derive(Clone, Copy)]
enum Action {
    KdeShortcut(&'static str), 
    CloseWindow,
    SmartAudioSwap,
    AutoTile,
    ChaosTile, 
    CustomTile(TileMode), 
    PrevPage,             
    NextPage,             
}

#[derive(Debug, Clone)]
struct Window {
    id: String, title: String,
    x: i32, y: i32, width: i32, height: i32,
}

#[derive(Debug, Clone)]
struct Monitor {
    x: i32, y: i32, width: i32, height: i32,
    ux: i32, uy: i32, uw: i32, uh: i32, 
}

fn get_active_window() -> String {
    if let Ok(out) = Command::new("kdotool").arg("getactivewindow").output() {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        String::new()
    }
}

fn get_windows_and_monitors() -> (Vec<Window>, Vec<Monitor>) {
    let mut windows = Vec::new();
    let mut raw_monitors = Vec::new();
    let mut panels = Vec::new(); 
    
    let search_output = match Command::new("kdotool").arg("search").arg(".*").output() {
        Ok(out) => out, Err(_) => return (windows, Vec::new()), 
    };
    
    for id_line in String::from_utf8_lossy(&search_output.stdout).lines() {
        let id = id_line.trim();
        if id.is_empty() { continue; }

        // THE FIX: We split the command and the string conversion into two lines so the memory stays alive!
        let title_out = Command::new("kdotool").args(["getwindowname", id]).output().unwrap();
        let title = String::from_utf8_lossy(&title_out.stdout).trim().to_string();

        let geom_out = Command::new("kdotool").args(["getwindowgeometry", id]).output().unwrap();
        let geom_str = String::from_utf8_lossy(&geom_out.stdout);

        let mut x = 0; let mut y = 0; let mut width = 0; let mut height = 0;
        for line in geom_str.lines() {
            let line = line.trim();
            if let Some(coords) = line.strip_prefix("Position:") {
                let parts: Vec<&str> = coords.split(',').map(|s| s.trim()).collect();
                if parts.len() == 2 { x = parts[0].parse().unwrap_or(0); y = parts[1].parse().unwrap_or(0); }
            } else if let Some(dims) = line.strip_prefix("Geometry:") {
                let parts: Vec<&str> = dims.split('x').map(|s| s.trim()).collect();
                if parts.len() == 2 { width = parts[0].parse().unwrap_or(0); height = parts[1].parse().unwrap_or(0); }
            }
        }

        let win = Window { id: id.to_string(), title: title.clone(), x, y, width, height };
        let lower_title = title.to_lowercase();

        if title.is_empty() || lower_title == "desktop — plasma" || lower_title == "plasma" {
            if width >= 800 && height >= 600 { raw_monitors.push(win); }
            continue; 
        }
        
        if title.contains("Panel") || height < 120 { 
            panels.push(win);
            continue; 
        }

        windows.push(win);
    }

    windows.sort_by(|a, b| (b.width * b.height).cmp(&(a.width * a.height)));
    raw_monitors.sort_by(|a, b| a.x.cmp(&b.x));

    let mut monitors = Vec::new();
    for rm in raw_monitors {
        let mut mon = Monitor { 
            x: rm.x, y: rm.y, width: rm.width, height: rm.height,
            ux: rm.x, uy: rm.y, uw: rm.width, uh: rm.height 
        };

        for p in &panels {
            if p.x >= mon.x && p.x < mon.x + mon.width {
                if p.y <= mon.y + 10 { 
                    mon.uy = mon.uy.max(p.y + p.height);
                    mon.uh = mon.height - (mon.uy - mon.y);
                } else if p.y + p.height >= mon.y + mon.height - 10 { 
                    mon.uh = mon.uh.min(p.y - mon.uy);
                }
            }
        }
        monitors.push(mon);
    }

    (windows, monitors)
}

fn execute_action(action: Action, target_win_id: &str) {
    match action {
        Action::KdeShortcut(shortcut) => { 
            if !target_win_id.is_empty() {
                let _ = Command::new("kdotool").args(["windowactivate", target_win_id]).output();
                std::thread::sleep(Duration::from_millis(50)); 
            }
            let _ = Command::new("dbus-send")
                .args(["--session", "--dest=org.kde.kglobalaccel", "--type=method_call", "/component/kwin", "org.kde.kglobalaccel.Component.invokeShortcut", &format!("string:{}", shortcut)])
                .output();
        }
        Action::CloseWindow => {
            if !target_win_id.is_empty() { let _ = Command::new("kdotool").args(["windowclose", target_win_id]).output(); }
        }
        _ => {} 
    }
}

fn render_offset_button(
    frame: &mut ratatui::Frame, text: &str, base_rect: Rect, offset_x: i32, bounds: Rect, action: Action, click_map: &mut Vec<(Rect, Action)>
) {
    let theoretical_x = base_rect.x as i32 + offset_x;
    let mut new_rect = base_rect;
    
    let x1 = theoretical_x.max(bounds.x as i32);
    let x2 = (theoretical_x + base_rect.width as i32).min((bounds.x + bounds.width) as i32);
    
    if x1 < x2 {
        new_rect.x = x1 as u16;
        new_rect.width = (x2 - x1) as u16;
        
        let btn_style = Style::default().fg(Color::White).bg(Color::DarkGray);
        let btn_block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded);
        let p = Paragraph::new(text.to_string()).alignment(Alignment::Center).style(btn_style).block(btn_block);
        
        frame.render_widget(p, new_rect);
        click_map.push((new_rect, action));
    }
}

fn main() -> io::Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableMouseCapture)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut last_refresh = Instant::now();
    let (mut windows, mut monitors) = get_windows_and_monitors();
    let mut active_window_id = get_active_window();
    
    let mut click_map_windows: Vec<(Rect, String)> = Vec::new();
    let mut click_map_buttons: Vec<(Rect, Action)> = Vec::new();
    
    let mut dragging_win: Option<(String, u16, u16)> = None;
    let mut last_drag_cmd = Instant::now(); 
    let mut last_clicked_win_id: Option<String> = None;
    let mut last_click_time = Instant::now();

    let mut current_page = 0;
    let mut target_page = 0;
    let mut anim_start = Instant::now();
    let mut is_animating = false;
    let total_pages = 2; 

    loop {
        if !is_animating && dragging_win.is_none() && last_refresh.elapsed() > Duration::from_millis(500) {
            let data = get_windows_and_monitors();
            windows = data.0; monitors = data.1;
            active_window_id = get_active_window();
            last_refresh = Instant::now();
        }

        let mut min_x = 0; let mut min_y = 0;
        let mut max_x = 1920; let mut max_y = 1080; 
        for m in &monitors {
            if m.x < min_x { min_x = m.x; }
            if m.y < min_y { min_y = m.y; }
            if m.x + m.width > max_x { max_x = m.x + m.width; }
            if m.y + m.height > max_y { max_y = m.y + m.height; }
        }
        let desktop_width = (max_x - min_x).max(1) as f64;
        let desktop_height = (max_y - min_y).max(1) as f64;

        let mut main_view_rect = Rect::default();

        terminal.draw(|frame| {
            click_map_windows.clear();
            click_map_buttons.clear();
            let area = frame.area();

            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(5), Constraint::Length(8)]) 
                .split(area);
            
            main_view_rect = main_chunks[0];

            for (index, mon) in monitors.iter().enumerate() {
                let term_x = (((mon.x - min_x) as f64 / desktop_width) * main_chunks[0].width as f64).round() as u16;
                let term_y = (((mon.y - min_y) as f64 / desktop_height) * main_chunks[0].height as f64).round() as u16;
                let term_w = ((mon.width as f64 / desktop_width) * main_chunks[0].width as f64).round() as u16; 
                let term_h = ((mon.height as f64 / desktop_height) * main_chunks[0].height as f64).round() as u16; 

                let rect = Rect::new(main_chunks[0].x + term_x, main_chunks[0].y + term_y, term_w, term_h);
                let mon_title = format!(" 🖥️ Display {} ", index + 1);
                let block = Block::default().title(mon_title).title_alignment(Alignment::Center)
                    .borders(Borders::ALL).border_type(BorderType::Double).border_style(Style::default().fg(Color::DarkGray)); 

                frame.render_widget(Paragraph::new("").block(block), rect);
            }

            for win in &windows {
                let term_x = (((win.x - min_x) as f64 / desktop_width) * main_chunks[0].width as f64).round() as u16;
                let term_y = (((win.y - min_y) as f64 / desktop_height) * main_chunks[0].height as f64).round() as u16;
                let term_w = ((win.width as f64 / desktop_width) * main_chunks[0].width as f64).max(8.0).round() as u16; 
                let term_h = ((win.height as f64 / desktop_height) * main_chunks[0].height as f64).max(4.0).round() as u16; 

                let rect_x = term_x.min(main_chunks[0].width.saturating_sub(1));
                let rect_y = term_y.min(main_chunks[0].height.saturating_sub(1));
                let rect_w = term_w.min(main_chunks[0].width - rect_x);
                let rect_h = term_h.min(main_chunks[0].height - rect_y);

                let rect = Rect::new(main_chunks[0].x + rect_x, main_chunks[0].y + rect_y, rect_w, rect_h);
                click_map_windows.push((rect, win.id.clone()));

                let is_active = win.id == active_window_id;
                let border_color = if let Some((ref dragging_id, _, _)) = dragging_win {
                    if dragging_id == &win.id { Color::Yellow } else if is_active { Color::Green } else { Color::White }
                } else if is_active { Color::Green } else { Color::White };
                
                let text_style = if is_active { Style::default().fg(Color::White).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::Gray) };
                let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(border_color)); 
                let inner_text = format!(" {}", win.title);
                let paragraph = Paragraph::new(inner_text).block(block).alignment(Alignment::Left).style(text_style);

                frame.render_widget(paragraph, rect);
            }

            let toolbar_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(4), Constraint::Min(10), Constraint::Length(4)])
                .split(main_chunks[1]);
            
            let left_btn_rect = toolbar_layout[0];
            let carousel_rect = toolbar_layout[1];
            let right_btn_rect = toolbar_layout[2];

            let nav_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
            let left_block = Paragraph::new("\n<").alignment(Alignment::Center).style(nav_style).block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
            let right_block = Paragraph::new("\n>").alignment(Alignment::Center).style(nav_style).block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
            frame.render_widget(left_block, left_btn_rect);
            frame.render_widget(right_block, right_btn_rect);
            click_map_buttons.push((left_btn_rect, Action::PrevPage));
            click_map_buttons.push((right_btn_rect, Action::NextPage));

            let progress = if is_animating { (anim_start.elapsed().as_secs_f32() / 0.25).min(1.0) } else { 1.0 };
            if progress >= 1.0 { is_animating = false; current_page = target_page; }
            let eased = 1.0 - (1.0 - progress).powi(3);
            
            let slide_dir = target_page as i32 - current_page as i32;
            let offset_x = (eased * carousel_rect.width as f32) as i32 * slide_dir.signum();

            let page_rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(carousel_rect);
            let grid_chunks_r1 = Layout::default().direction(Direction::Horizontal).constraints(vec![Constraint::Ratio(1, 4); 4]).split(page_rows[0]);
            let grid_chunks_r2 = Layout::default().direction(Direction::Horizontal).constraints(vec![Constraint::Ratio(1, 4); 4]).split(page_rows[1]);

            let p0_r1 = [("◧ LEFT HALF", Action::CustomTile(TileMode::LeftHalf)), ("◨ RIGHT HALF", Action::CustomTile(TileMode::RightHalf)), ("◰ TOP LEFT", Action::CustomTile(TileMode::TopLeft)), ("◳ TOP RIGHT", Action::CustomTile(TileMode::TopRight))];
            let p0_r2 = [("◲ BOT LEFT", Action::CustomTile(TileMode::BotLeft)), ("◱ BOT RIGHT", Action::CustomTile(TileMode::BotRight)), ("📺 TO LEFT MON", Action::KdeShortcut("Window One Screen to the Left")), ("TO RIGHT MON 📺", Action::KdeShortcut("Window One Screen to the Right"))];

            let p1_r1 = [("🗗 TOGGLE MAX", Action::KdeShortcut("Window Maximize")), ("🔲 FULLSCREEN", Action::KdeShortcut("Window Fullscreen")), ("🔽 MINIMIZE", Action::KdeShortcut("Window Minimize")), ("❌ CLOSE", Action::CloseWindow)];
            let p1_r2 = [("⊞ AUTO TILE", Action::AutoTile), ("🎲 CHAOS TILE", Action::ChaosTile), ("🔄 SWAP SCREENS", Action::SmartAudioSwap), ("🎵 AUDIO SWAP", Action::SmartAudioSwap)];

            let pages_to_draw = if is_animating { vec![current_page, target_page] } else { vec![current_page] };
            
            for &page in &pages_to_draw {
                let page_offset = if page == current_page { -offset_x } else { (carousel_rect.width as i32 * slide_dir.signum()) - offset_x };
                
                let (r1_data, r2_data) = if page == 0 { (p0_r1, p0_r2) } else { (p1_r1, p1_r2) };

                for i in 0..4 {
                    render_offset_button(frame, r1_data[i].0, grid_chunks_r1[i], page_offset, carousel_rect, r1_data[i].1, &mut click_map_buttons);
                    render_offset_button(frame, r2_data[i].0, grid_chunks_r2[i], page_offset, carousel_rect, r2_data[i].1, &mut click_map_buttons);
                }
            }
        })?;

        if event::poll(Duration::from_millis(15))? {
            match event::read()? {
                event::Event::Key(key) => { if key.code == KeyCode::Char('q') { break; } }
                event::Event::Mouse(mouse_event) => {
                    let tap_x = mouse_event.column;
                    let tap_y = mouse_event.row;
                    
                    match mouse_event.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            if is_animating { continue; }

                            let mut clicked_window = false;
                            
                            for (rect, win_id) in click_map_windows.iter().rev() {
                                if tap_x >= rect.x && tap_x < rect.x + rect.width && tap_y >= rect.y && tap_y < rect.y + rect.height {
                                    
                                    let now = Instant::now();
                                    if let Some(ref last_id) = last_clicked_win_id {
                                        if last_id == win_id && now.duration_since(last_click_time) < Duration::from_millis(750) {
                                            let win_id_clone = win_id.clone();
                                            std::thread::spawn(move || {
                                                let _ = Command::new("kdotool").args(["windowactivate", &win_id_clone]).output();
                                                std::thread::sleep(Duration::from_millis(150)); 
                                                let _ = Command::new("dbus-send").args(["--session", "--dest=org.kde.kglobalaccel", "--type=method_call", "/component/kwin", "org.kde.kglobalaccel.Component.invokeShortcut", "string:Window Maximize"]).output();
                                            });
                                            last_clicked_win_id = None; 
                                            clicked_window = true;
                                            break;
                                        }
                                    }
                                    
                                    last_clicked_win_id = Some(win_id.clone());
                                    last_click_time = now;

                                    let _ = Command::new("kdotool").args(["windowactivate", win_id]).output();
                                    dragging_win = Some((win_id.clone(), tap_x.saturating_sub(rect.x), tap_y.saturating_sub(rect.y)));
                                    
                                    clicked_window = true;
                                    let data = get_windows_and_monitors(); windows = data.0; monitors = data.1;
                                    active_window_id = get_active_window();
                                    last_refresh = Instant::now();
                                    break; 
                                }
                            }

                            if !clicked_window {
                                for (rect, action) in &click_map_buttons {
                                    if tap_x >= rect.x && tap_x < rect.x + rect.width && tap_y >= rect.y && tap_y < rect.y + rect.height {
                                        match action {
                                            Action::PrevPage => { if current_page > 0 { target_page = current_page - 1; is_animating = true; anim_start = Instant::now(); } },
                                            Action::NextPage => { if current_page < total_pages - 1 { target_page = current_page + 1; is_animating = true; anim_start = Instant::now(); } },
                                            Action::SmartAudioSwap => {
                                                let mut hdmi_sink = String::new(); let mut desk_sink = String::new();
                                                if let Ok(out) = Command::new("pactl").args(["list", "short", "sinks"]).output() {
                                                    for line in String::from_utf8_lossy(&out.stdout).lines() {
                                                        let parts: Vec<&str> = line.split_whitespace().collect();
                                                        if parts.len() >= 2 {
                                                            let s = parts[1].to_string();
                                                            if s.contains("hdmi") { hdmi_sink = s.clone(); }
                                                            else if s.contains("Focusrite") { desk_sink = s.clone(); }
                                                            else if desk_sink.is_empty() && s.contains("usb") { desk_sink = s.clone(); } 
                                                        }
                                                    }
                                                }
                                                if let Ok(out) = Command::new("pactl").arg("get-default-sink").output() {
                                                    let cur = String::from_utf8_lossy(&out.stdout);
                                                    if cur.contains("hdmi") && !desk_sink.is_empty() { let _ = Command::new("pactl").args(["set-default-sink", &desk_sink]).output(); } 
                                                    else if !hdmi_sink.is_empty() { let _ = Command::new("pactl").args(["set-default-sink", &hdmi_sink]).output(); }
                                                }
                                            },
                                            Action::CustomTile(mode) => {
                                                let mut target_mon = monitors.first().cloned();
                                                if let Some(awin) = windows.iter().find(|w| w.id == active_window_id) {
                                                    let cx = awin.x + (awin.width / 2);
                                                    for m in &monitors { if cx >= m.x && cx <= m.x + m.width { target_mon = Some(m.clone()); break; } }
                                                }
                                                if let Some(mon) = target_mon {
                                                    let gap = 16;
                                                    let hw = (mon.uw - gap*3)/2; 
                                                    let hh = (mon.uh - gap*3)/2; let fh = mon.uh - gap*2;
                                                    let xl = mon.ux + gap; let xr = mon.ux + gap*2 + hw;
                                                    let yt = mon.uy + gap; let yb = mon.uy + gap*2 + hh;

                                                    let (nx, ny, nw, nh) = match mode {
                                                        TileMode::LeftHalf => (xl, yt, hw, fh),
                                                        TileMode::RightHalf => (xr, yt, hw, fh),
                                                        TileMode::TopLeft => (xl, yt, hw, hh),
                                                        TileMode::TopRight => (xr, yt, hw, hh),
                                                        TileMode::BotLeft => (xl, yb, hw, hh),
                                                        TileMode::BotRight => (xr, yb, hw, hh),
                                                    };
                                                    
                                                    let _ = Command::new("kdotool").args(["windowmove", &active_window_id, &nx.to_string(), &ny.to_string()]).output();
                                                    let _ = Command::new("kdotool").args(["windowsize", &active_window_id, &nw.to_string(), &nh.to_string()]).output();
                                                    let _ = Command::new("kdotool").args(["windowmove", &active_window_id, &nx.to_string(), &ny.to_string()]).output();
                                                }
                                            },
                                            Action::AutoTile | Action::ChaosTile => {
                                                if monitors.is_empty() { break; }
                                                let is_chaos = matches!(action, Action::ChaosTile);
                                                let mut monitor_assignments: Vec<Vec<&Window>> = vec![Vec::new(); monitors.len()];
                                                let tileable_windows: Vec<&Window> = windows.iter().filter(|w| w.width > 250 && w.height > 250).collect();

                                                if is_chaos {
                                                    let mut seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos().max(1);
                                                    for win in &tileable_windows {
                                                        seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5;
                                                        monitor_assignments[(seed as usize) % monitors.len()].push(win);
                                                    }
                                                } else {
                                                    for win in &tileable_windows {
                                                        let cx = win.x + (win.width / 2);
                                                        for (i, m) in monitors.iter().enumerate() {
                                                            if cx >= m.x && cx <= m.x + m.width { monitor_assignments[i].push(win); break; }
                                                        }
                                                    }
                                                }

                                                for (i, mon) in monitors.iter().enumerate() {
                                                    let local_wins = &monitor_assignments[i];
                                                    let n = local_wins.len();
                                                    let gap = 16; 
                                                    let hw = (mon.uw - gap*3)/2; let fw = mon.uw - gap*2;
                                                    let hh = (mon.uh - gap*3)/2; let fh = mon.uh - gap*2;
                                                    let xl = mon.ux + gap; let xr = mon.ux + gap*2 + hw;
                                                    let yt = mon.uy + gap; let yb = mon.uy + gap*2 + hh;
                                                    
                                                    let limit = n.min(4); 
                                                    for (j, win) in local_wins.iter().enumerate() {
                                                        let (nx, ny, nw, nh) = match (limit, j) {
                                                            (1, 0) => (xl, yt, fw, fh),
                                                            (2, 0) => (xl, yt, hw, fh), (2, 1) => (xr, yt, hw, fh),
                                                            (3, 0) => (xl, yt, hw, fh), (3, 1) => (xr, yt, hw, hh), (3, 2) => (xr, yb, hw, hh),
                                                            (4, 0) => (xl, yt, hw, hh), (4, 1) => (xr, yt, hw, hh), (4, 2) => (xl, yb, hw, hh),
                                                            _ => (xr, yb, hw, hh), 
                                                        };
                                                        let _ = Command::new("kdotool").args(["windowmove", &win.id, &nx.to_string(), &ny.to_string()]).output();
                                                        let _ = Command::new("kdotool").args(["windowsize", &win.id, &nw.to_string(), &nh.to_string()]).output();
                                                        let _ = Command::new("kdotool").args(["windowmove", &win.id, &nx.to_string(), &ny.to_string()]).output();
                                                    }
                                                }
                                            },
                                            _ => execute_action(*action, &active_window_id) 
                                        }
                                        
                                        let data = get_windows_and_monitors(); windows = data.0; monitors = data.1;
                                        active_window_id = get_active_window();
                                        last_refresh = Instant::now();
                                        break;
                                    }
                                }
                            }
                        },
                        
                        MouseEventKind::Drag(MouseButton::Left) => {
                            if let Some((ref win_id, off_x, off_y)) = dragging_win {
                                let term_w = main_view_rect.width as f64; let term_h = main_view_rect.height as f64;
                                if term_w > 0.0 && term_h > 0.0 {
                                    let local_x = tap_x.saturating_sub(main_view_rect.x).saturating_sub(off_x) as f64;
                                    let local_y = tap_y.saturating_sub(main_view_rect.y).saturating_sub(off_y) as f64;
                                    let new_desk_x = min_x as f64 + (local_x / term_w) * desktop_width;
                                    let new_desk_y = min_y as f64 + (local_y / term_h) * desktop_height;

                                    if last_drag_cmd.elapsed() > Duration::from_millis(32) {
                                        let _ = Command::new("kdotool").args(["windowmove", win_id, &new_desk_x.round().to_string(), &new_desk_y.round().to_string()]).spawn();
                                        last_drag_cmd = Instant::now();
                                    }
                                    if let Some(win) = windows.iter_mut().find(|w| &w.id == win_id) { win.x = new_desk_x.round() as i32; win.y = new_desk_y.round() as i32; }
                                }
                            }
                        },
                        
                        MouseEventKind::Up(MouseButton::Left) => {
                            if dragging_win.is_some() {
                                dragging_win = None;
                                let data = get_windows_and_monitors(); windows = data.0; monitors = data.1;
                                active_window_id = get_active_window();
                                last_refresh = Instant::now();
                            }
                        },
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    stdout().execute(DisableMouseCapture)?;
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
