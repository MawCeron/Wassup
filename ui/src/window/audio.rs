//! Playing audio and voice notes.

use super::*;
use gettextrs::gettext;

impl Window {
    /// Plays or pauses a message's audio, downloading it first if needed.
    pub(super) fn toggle_audio(self: &Rc<Self>, msg: &ipc::Message) {
        let Some(media) = &msg.media else { return };
        let current = self
            .player
            .borrow()
            .as_ref()
            .filter(|p| p.msg_id == msg.id)
            .map(|p| p.media.clone());
        if let Some(stream) = current {
            stream.set_playing(!stream.is_playing());
        } else if media.path.is_empty() {
            self.play_when_ready.borrow_mut().insert(msg.id.clone());
            self.request_media(msg);
        } else {
            self.start_audio(&msg.id, &media.path);
        }
    }

    pub(super) fn start_audio(self: &Rc<Self>, msg_id: &str, path: &str) {
        self.stop_audio();
        let duration = (0..self.messages.n_items())
            .filter_map(|i| self.messages.item(i).and_downcast::<glib::BoxedAnyObject>())
            .find_map(|item| {
                let m = item.borrow::<ipc::Message>();
                (m.id == msg_id).then(|| m.media.as_ref().map_or(0, |a| a.duration_seconds))
            })
            .unwrap_or(0);
        let media = gtk::MediaFile::for_filename(path);
        let ui = Rc::downgrade(self);
        let refresh = {
            let ui = ui.clone();
            move || {
                if let Some(ui) = ui.upgrade() {
                    ui.show_audio_state();
                }
            }
        };
        media.connect_timestamp_notify({
            let refresh = refresh.clone();
            move |_| refresh()
        });
        media.connect_playing_notify(move |_| refresh());
        media.connect_ended_notify(move |stream| {
            if let Some(ui) = ui.upgrade().filter(|_| stream.is_ended()) {
                ui.stop_audio();
            }
        });
        // A plain closure, not glib::clone!, so xgettext finds the message.
        let weak = Rc::downgrade(self);
        media.connect_error_notify(move |stream| {
            if let (Some(ui), Some(e)) = (weak.upgrade(), stream.error()) {
                // TRANSLATORS: {} is the reason, from GStreamer.
                ui.toast(&gettext("Couldn't play the audio: {}").replace("{}", &e.to_string()));
                ui.stop_audio();
            }
        });
        media.play();
        self.player.replace(Some(Player {
            msg_id: msg_id.to_owned(),
            duration,
            media,
        }));
        self.show_audio_state();
    }

    /// Frees the player; its row goes back to the start.
    pub(super) fn stop_audio(&self) {
        let Some(player) = self.player.take() else {
            return;
        };
        player.media.pause();
        let row = self
            .rows
            .borrow()
            .get(&player.msg_id)
            .and_then(|r| r.audio.upgrade());
        if let Some(row) = row {
            show_audio(&row, player.duration, None);
        }
    }

    pub(super) fn seek_audio(&self, msg_id: &str, seconds: f64) {
        if let Some(player) = self.player.borrow().as_ref().filter(|p| p.msg_id == msg_id) {
            player.media.seek((seconds * 1_000_000.0) as i64);
        }
    }

    /// Updates the playing message's row, if it is on screen.
    pub(super) fn show_audio_state(&self) {
        let player = self.player.borrow();
        let Some(player) = player.as_ref() else {
            return;
        };
        let row = self
            .rows
            .borrow()
            .get(&player.msg_id)
            .and_then(|r| r.audio.upgrade());
        if let Some(row) = row {
            show_audio(&row, player.duration, Some(&player.media));
        }
    }
}

/// Draws an audio row: play or pause, how far it got, and the time.
pub(super) fn show_audio(row: &gtk::Box, duration: u32, playing: Option<&gtk::MediaFile>) {
    let play = row
        .first_child()
        .and_then(|w| w.next_sibling())
        .and_downcast::<gtk::Button>()
        .unwrap();
    let progress = play.next_sibling().and_downcast::<gtk::Scale>().unwrap();
    let length = progress
        .next_sibling()
        .and_downcast::<gtk::Label>()
        .unwrap();
    let total = match playing.map(|m| m.duration()) {
        Some(us) if us > 0 => us as f64 / 1e6,
        _ => f64::from(duration.max(1)),
    };
    let at = playing.map_or(0.0, |m| m.timestamp() as f64 / 1e6);
    let running = playing.is_some_and(|m| m.is_playing());
    play.set_icon_name(if running {
        "wassup-pause-symbolic"
    } else {
        "wassup-play-symbolic"
    });
    play.set_tooltip_text(Some(&if running {
        gettext("Pause")
    } else {
        gettext("Play")
    }));
    progress.set_range(0.0, total);
    progress.set_value(at);
    length.set_label(&clock(if playing.is_some() {
        at
    } else {
        f64::from(duration)
    }));
}

/// "m:ss", the way chat apps show audio lengths.
pub(super) fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_clock() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(7.9), "0:07");
        assert_eq!(clock(125.0), "2:05");
    }
}
