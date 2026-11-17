use crate::api::models::ulid_timestamp;
use crate::state::{AppState, Focus};
use crate::ui::markdown::MarkdownParser;
use chrono::{Datelike, Local};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

pub struct ChatFeed<'a> {
    state: &'a mut AppState,
}

impl<'a> ChatFeed<'a> {
    pub fn new(state: &'a mut AppState) -> Self {
        Self { state }
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn format_ulid_time(id: &str) -> String {
    if let Some(utc) = ulid_timestamp(id) {
        let local = utc.with_timezone(&Local);
        let now = Local::now();
        if local.year() == now.year() && local.ordinal() == now.ordinal() {
            local.format("%H:%M").to_string()
        } else {
            local.format("%d/%m %H:%M").to_string()
        }
    } else {
        "--:--".to_string()
    }
}

fn format_system_event(system: &serde_json::Value) -> String {
    let event_type = system.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match event_type {
        "user_added" => {
            let u = system.get("user").and_then(|v| v.as_str()).unwrap_or("someone");
            format!("→ User {} was added to the channel", u)
        }
        "user_remove" => {
            let u = system.get("user").and_then(|v| v.as_str()).unwrap_or("someone");
            format!("← User {} was removed from the channel", u)
        }
        "user_joined" => {
            let u = system.get("id").or_else(|| system.get("user")).and_then(|v| v.as_str()).unwrap_or("someone");
            format!("→ User {} joined the server", u)
        }
        "user_left" => {
            let u = system.get("id").or_else(|| system.get("user")).and_then(|v| v.as_str()).unwrap_or("someone");
            format!("← User {} left the server", u)
        }
        "user_kicked" => {
            let u = system.get("id").or_else(|| system.get("user")).and_then(|v| v.as_str()).unwrap_or("someone");
            format!("🔨 User {} was kicked", u)
        }
        "user_banned" => {
            let u = system.get("id").or_else(|| system.get("user")).and_then(|v| v.as_str()).unwrap_or("someone");
            format!("⛔ User {} was banned", u)
        }
        "channel_renamed" => {
            let name = system.get("name").and_then(|v| v.as_str()).unwrap_or("channel");
            format!("✏️ Channel renamed to #{}", name)
        }
        "channel_description_changed" => "📝 Channel description updated".to_string(),
        "channel_icon_changed" => "🖼️ Channel icon updated".to_string(),
        "call_started" => "📞 Call started in channel".to_string(),
        other => {
            if !other.is_empty() {
                format!("ℹ️ System event: {}", other)
            } else {
                "ℹ️ System notice".to_string()
            }
        }
    }
}

enum ChatRenderItem {
    Line(Line<'static>),
    AuthorHeader {
        spans: Vec<Span<'static>>,
        author_id: String,
    },
    ImageAttachment {
        key: String,
        badge: String,
        height: u16,
    },
}

impl<'a> Widget for ChatFeed<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let is_focused = self.state.focus == Focus::Chat;
        let theme = self.state.theme.clone();
        let parser = MarkdownParser::default();
        let is_compact = area.width < 24 || area.height < 8;

        // Fill background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.bg);
            }
        }

        // Draw border if focused (non-compact only; Borders::NONE is a no-op anyway)
        if is_focused && !is_compact {
            let block = Block::default()
                .borders(Borders::NONE)
                .border_style(theme.focused_border_style(is_focused));
            block.render(area, buf);
        }

        if area.width < 2 || area.height < 1 {
            return;
        }

        let pad_x: u16 = if is_compact && area.width <= 14 { 0 } else { 1 };
        let line_x = area.left() + pad_x;
        let max_line_w = area.width.saturating_sub(pad_x.saturating_mul(2));

        let channel_id = match &self.state.active_channel_id {
            Some(cid) => cid,
            None => {
                let msg = if is_compact { "No ch" } else { "No channel selected. Select a channel from the left sidebar." };
                let y = area.top() + if area.height >= 4 { 1 } else { 0 };
                if y < area.bottom() && max_line_w > 0 {
                    buf.set_line(line_x, y, &Line::from(Span::styled(msg, Style::default().fg(theme.muted))), max_line_w);
                }
                return;
            }
        };

        let messages = self.state.messages.get(channel_id);
        let empty_vec = Vec::new();
        let msgs = messages.unwrap_or(&empty_vec);

        if msgs.is_empty() {
            let ch_name = self.state.active_channel().map(|c| c.display_name(&self.state.users, self.state.current_user.as_ref().map(|u| u.id.as_str()))).unwrap_or_else(|| "this channel".to_string());
            let y1 = area.top() + if area.height >= 4 { 1 } else { 0 };
            if y1 < area.bottom() && max_line_w > 0 {
                let title = if is_compact { format!("#{}", ch_name) } else { format!("Welcome to #{}!", ch_name) };
                buf.set_line(
                    line_x,
                    y1,
                    &Line::from(Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
                    max_line_w,
                );
            }
            let y2 = y1 + 1;
            if y2 < area.bottom() && max_line_w > 0 {
                let sub = if is_compact { "No msgs" } else { "This is the start of the channel history. Send a message below to begin chatting!" };
                buf.set_line(
                    line_x,
                    y2,
                    &Line::from(Span::styled(sub, Style::default().fg(theme.muted))),
                    max_line_w,
                );
            }
            return;
        }

        // Pre-render all messages into formatted items
        let mut rendered_items: Vec<ChatRenderItem> = Vec::new();
        let mut prev_author: Option<String> = None;
        let mut prev_ts_ms: Option<u64> = None;

        for msg in msgs {
            let has_content = msg.content.as_ref().map(|c| !c.trim().is_empty()).unwrap_or(false);
            let has_attachments = msg.attachments.as_ref().map(|a| !a.is_empty()).unwrap_or(false);
            let has_embeds = msg.embeds.as_ref().map(|e| !e.is_empty()).unwrap_or(false);
            let is_system = msg.author == "00000000000000000000000000" || msg.system.is_some();

            // Skip rendering completely empty ghost messages
            if !has_content && !has_attachments && !has_embeds && !is_system {
                continue;
            }

            let time_str = format_ulid_time(&msg.id);
            let msg_ts_ms = crate::api::models::ulid_timestamp(&msg.id)
                .map(|dt| dt.timestamp_millis().max(0) as u64);

            // Handle system messages specially with subtle muted rule
            if is_system {
                prev_author = None;
                prev_ts_ms = None;
                let notice = if let Some(ref sys) = msg.system {
                    format_system_event(sys)
                } else if let Some(ref c) = msg.content {
                    c.clone()
                } else {
                    "ℹ️ System event".to_string()
                };

                let sys_spans = vec![
                    Span::styled("─── ", Style::default().fg(Color::Rgb(62, 65, 74))),
                    Span::styled(
                        format!("{} • {}", notice, time_str),
                        Style::default().fg(Color::Rgb(150, 155, 170)),
                    ),
                    Span::styled(" ───", Style::default().fg(Color::Rgb(62, 65, 74))),
                ];
                rendered_items.push(ChatRenderItem::Line(Line::from(sys_spans)));
                rendered_items.push(ChatRenderItem::Line(Line::from(String::new())));
                continue;
            }

            // Consecutive message grouping: same author within 5 minutes (when not compact_mode)
            let is_grouped = !is_compact
                && msg.masquerade.is_none()
                && prev_author.as_deref() == Some(msg.author.as_str())
                && match (prev_ts_ms, msg_ts_ms) {
                    (Some(p), Some(c)) => c.saturating_sub(p) <= 5 * 60 * 1000,
                    _ => false,
                };

            prev_author = Some(msg.author.clone());
            prev_ts_ms = msg_ts_ms;

            if !is_grouped {
                // 1. Author header line (resolved from cache or fetched asynchronously)
                let user = self.state.resolve_user(&msg.author).cloned();
                let author_name = if let Some(ref m) = msg.masquerade {
                    m.name.clone().unwrap_or_else(|| "Anonymous".to_string())
                } else if let Some(ref u) = user {
                    u.name().to_string()
                } else {
                    format!("User {}", &msg.author[..msg.author.len().min(6)])
                };

                let author_color = theme.role_color(&msg.author);

                let mut header_spans = if is_compact {
                    vec![Span::styled(author_name, Style::default().fg(author_color).add_modifier(Modifier::BOLD))]
                } else {
                    vec![
                        Span::styled(format!("[{}] ", time_str), Style::default().fg(Color::Rgb(95, 98, 110))),
                        Span::styled(author_name, Style::default().fg(author_color).add_modifier(Modifier::BOLD)),
                    ]
                };

                // Bot badge
                if !is_compact {
                    if let Some(ref u) = user {
                        if u.bot.is_some() {
                            header_spans.push(Span::styled(" BOT", Style::default().fg(Color::White).bg(theme.accent).add_modifier(Modifier::BOLD)));
                        }
                    }
                }

                rendered_items.push(ChatRenderItem::AuthorHeader {
                    spans: header_spans,
                    author_id: msg.author.clone(),
                });
            }

            // 2. Content lines
            let content = msg.content.as_deref().unwrap_or("");
            if !content.trim().is_empty() {
                let content_lines = parser.parse_message(content);
                for mut cl in content_lines {
                    if is_compact {
                        let mut prefixed = vec![Span::styled(" ", Style::default())];
                        prefixed.append(&mut cl.spans);
                        rendered_items.push(ChatRenderItem::Line(Line::from(prefixed)));
                    } else {
                        let mut indented = vec![Span::styled("    ", Style::default())];
                        indented.append(&mut cl.spans);
                        rendered_items.push(ChatRenderItem::Line(Line::from(indented)));
                    }
                }
            }

            // 3. Attachments & Images
            if let Some(ref attachments) = msg.attachments {
                for att in attachments {
                    let name = att.filename.as_deref().unwrap_or("file");
                    let size_str = att.size.map(format_bytes).unwrap_or_else(|| "unknown size".to_string());
                    let badge = format!("    📎 {} ({})", name, size_str);

                    // Check if attachment is an image
                    let is_image = att.content_type.as_deref().map(|c| c.starts_with("image/")).unwrap_or(false)
                        || att.metadata.as_ref().and_then(|m| m.file_type.as_deref()) == Some("Image")
                        || name.ends_with(".png")
                        || name.ends_with(".jpg")
                        || name.ends_with(".jpeg")
                        || name.ends_with(".webp")
                        || name.ends_with(".gif");

                    if is_image {
                        let key = format!("attachment_{}", att.id);
                        let url = self.state.attachment_image_url(&att.id);
                        let mut is_cached = false;
                        if let Some(ref mut cache) = self.state.image_cache {
                            if cache.contains(&key) {
                                is_cached = true;
                            } else if !cache.is_pending(&key) {
                                cache.mark_pending(key.clone());
                                if let Some(ref tx) = self.state.image_fetch_tx {
                                    let _ = tx.send((key.clone(), url));
                                }
                            }
                        }

                        if is_cached {
                            // Image height constrained to 10 rows
                            rendered_items.push(ChatRenderItem::ImageAttachment {
                                key: key.clone(),
                                badge: badge.clone(),
                                height: 10,
                            });
                        } else {
                            rendered_items.push(ChatRenderItem::Line(Line::from(vec![
                                Span::styled(badge.clone(), Style::default().fg(Color::Rgb(100, 200, 255)).add_modifier(Modifier::BOLD)),
                            ])));
                            rendered_items.push(ChatRenderItem::Line(Line::from(vec![
                                Span::styled("    [🖼️ Loading image...]", Style::default().fg(Color::Rgb(140, 140, 150)).add_modifier(Modifier::ITALIC)),
                            ])));
                        }
                    } else {
                        rendered_items.push(ChatRenderItem::Line(Line::from(vec![
                            Span::styled(badge, Style::default().fg(Color::Rgb(100, 200, 255)).add_modifier(Modifier::BOLD)),
                        ])));
                    }
                }
            }

            // 4. Embeds
            if let Some(ref embeds) = msg.embeds {
                for emb in embeds {
                    let title = emb.get("title").and_then(|t| t.as_str()).unwrap_or("Embed").to_string();
                    let url = emb.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
                    let desc = emb.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();
                    let mut embed_spans = vec![
                        Span::styled("    ┌ ", Style::default().fg(Color::Rgb(120, 120, 150))),
                        Span::styled(title, Style::default().fg(Color::Rgb(220, 220, 255)).add_modifier(Modifier::BOLD)),
                    ];
                    if !url.is_empty() {
                        embed_spans.push(Span::styled(format!(" ({})", url), Style::default().fg(Color::Rgb(100, 150, 240))));
                    }
                    rendered_items.push(ChatRenderItem::Line(Line::from(embed_spans)));
                    if !desc.is_empty() {
                        rendered_items.push(ChatRenderItem::Line(Line::from(vec![
                            Span::styled("    │ ", Style::default().fg(Color::Rgb(120, 120, 150))),
                            Span::styled(desc, Style::default().fg(Color::Rgb(180, 180, 190))),
                        ])));
                    }
                }
            }

            // 5. Reactions
            if let Some(ref reactions) = msg.reactions {
                if !reactions.is_empty() {
                    let mut rx_spans = vec![Span::styled("    ", Style::default())];
                    for (emoji, uids) in reactions {
                        let rx_str = format!(" [ {} {} ] ", emoji, uids.len());
                        rx_spans.push(Span::styled(
                            rx_str,
                            Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 50)),
                        ));
                    }
                    rendered_items.push(ChatRenderItem::Line(Line::from(rx_spans)));
                }
            }

            // Message separator spacing (skipped in compact/dense chat mode)
            if !self.state.config.compact_mode {
                rendered_items.push(ChatRenderItem::Line(Line::from(String::new())));
            }
        }

        // Expand items to lines count for scrolling calculation
        struct PositionedItem<'b> {
            y_offset: usize,
            height: usize,
            item: &'b ChatRenderItem,
        }

        let mut positioned_items = Vec::new();
        let mut total_lines = 0;
        for item in &rendered_items {
            let h = match item {
                ChatRenderItem::Line(_) => 1,
                ChatRenderItem::AuthorHeader { .. } => 1,
                ChatRenderItem::ImageAttachment { height, .. } => *height as usize + 1, // image + badge line
            };
            positioned_items.push(PositionedItem {
                y_offset: total_lines,
                height: h,
                item,
            });
            total_lines += h;
        }

        let visible_height = area.height as usize;
        let offset = self.state.chat_scroll_offset.min(total_lines.saturating_sub(visible_height));
        let view_start = total_lines.saturating_sub(offset + visible_height);
        let view_end = total_lines.saturating_sub(offset);

        for pos in &positioned_items {
            let item_top = pos.y_offset;
            let item_bottom = pos.y_offset + pos.height;

            // Check if item overlaps viewport [view_start, view_end)
            if item_bottom <= view_start || item_top >= view_end {
                continue;
            }

            match pos.item {
                ChatRenderItem::Line(line) => {
                    let screen_y = area.top() + (item_top.saturating_sub(view_start)) as u16;
                    if screen_y < area.bottom() && max_line_w > 0 {
                        buf.set_line(line_x, screen_y, line, max_line_w);
                    }
                }
                ChatRenderItem::AuthorHeader { spans, author_id } => {
                    let screen_y = area.top() + (item_top.saturating_sub(view_start)) as u16;
                    if screen_y < area.bottom() {
                        if !is_compact && area.width >= 10 {
                            // Render 2x1 PFP thumbnail on the left
                            let pfp_rect = Rect {
                                x: area.left() + 1,
                                y: screen_y,
                                width: 2,
                                height: 1,
                            };

                            // Fallback initials badge
                            let initials: String = self.state.users.get(author_id)
                                .map(|u| u.name())
                                .unwrap_or(author_id.as_str())
                                .chars()
                                .take(2)
                                .collect::<String>()
                                .to_uppercase();
                            let (badge_bg, badge_fg) = theme.user_initials_color(author_id);
                            let mut init_chars = initials.chars();
                            let c1 = init_chars.next().unwrap_or('?');
                            let c2 = init_chars.next().unwrap_or(' ');
                            crate::ui::put_cell(buf, area, area.left() + 1, screen_y, c1, Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));
                            crate::ui::put_cell(buf, area, area.left() + 2, screen_y, c2, Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));

                            let acc_id = self.state.account_for_user(author_id).to_string();
                            let key = self.state.avatar_cache_key(&acc_id, author_id, "avatar");
                            self.state.deferred_image_renders.push((pfp_rect, key.clone()));
                            self.state.request_user_avatar_with_key_in_account(&acc_id, author_id, &key);

                            let header_line = Line::from(spans.clone());
                            buf.set_line(area.left() + 4, screen_y, &header_line, area.width.saturating_sub(5));
                        } else if max_line_w > 0 {
                            let header_line = Line::from(spans.clone());
                            buf.set_line(line_x, screen_y, &header_line, max_line_w);
                        }
                    }
                }
                ChatRenderItem::ImageAttachment { key, badge, height } => {
                    let badge_line = Line::from(vec![
                        Span::styled(badge.clone(), Style::default().fg(Color::Rgb(100, 200, 255)).add_modifier(Modifier::BOLD)),
                    ]);

                    let img_screen_y = area.top() + (item_top.saturating_sub(view_start)) as u16;
                    let img_h = *height;

                    if img_screen_y + img_h <= area.bottom() {
                        let max_img_w = (area.width.saturating_sub(8)).min(60);
                        if max_img_w >= 2 && img_h >= 1 {
                            let img_rect = Rect {
                                x: area.left() + 4,
                                y: img_screen_y,
                                width: max_img_w,
                                height: img_h,
                            };

                            for iy in img_rect.top()..img_rect.bottom() {
                                for ix in img_rect.left()..img_rect.right() {
                                    crate::ui::put_cell(buf, area, ix, iy, ' ', Style::default().bg(Color::Rgb(30, 30, 35)));
                                }
                            }

                            self.state.deferred_image_renders.push((img_rect, key.clone()));
                        }
                    }

                    let badge_screen_y = img_screen_y + img_h;
                    if badge_screen_y < area.bottom() && max_line_w > 0 {
                        buf.set_line(line_x, badge_screen_y, &badge_line, max_line_w);
                    }
                }
            }
        }

        // Draw scrollbar indicator on the right edge if backlog is scrollable
        if total_lines > visible_height && area.width >= 6 {
            let scrollbar_x = area.right().saturating_sub(1);
            let track_height = area.height as usize;
            let thumb_height = ((visible_height as f32 / total_lines as f32) * track_height as f32).max(1.0) as usize;
            let max_scroll = total_lines.saturating_sub(visible_height);
            let thumb_pos = if max_scroll == 0 {
                0
            } else {
                ((max_scroll.saturating_sub(offset) as f32 / max_scroll as f32) * (track_height.saturating_sub(thumb_height)) as f32) as usize
            };

            for y_off in 0..track_height {
                let y = area.top() + y_off as u16;
                let is_thumb = y_off >= thumb_pos && y_off < thumb_pos + thumb_height;
                let ch = if is_thumb { '┃' } else { '│' };
                let color = if is_thumb {
                    Color::Rgb(120, 124, 136)
                } else {
                    Color::Rgb(52, 54, 62)
                };
                crate::ui::put_cell(buf, area, scrollbar_x, y, ch, Style::default().fg(color));
            }
        }
    }
}
