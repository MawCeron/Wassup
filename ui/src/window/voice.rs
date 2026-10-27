//! Recording and sending voice notes.

use super::*;
use gettextrs::gettext;

/// Recordings shorter than this are taken as a slip of the finger.
const MIN_RECORDING: std::time::Duration = std::time::Duration::from_secs(1);

/// The microphone button, and the bar that replaces the composer while
/// recording.
pub(super) struct VoiceBar {
    pub(super) mic: gtk::Button,
    pub(super) bar: gtk::Box,
    time: gtk::Label,
    cancel: gtk::Button,
    send: gtk::Button,
    recording: RefCell<Option<Recording>>,
}

struct Recording {
    process: gio::Subprocess,
    path: std::path::PathBuf,
    started: std::time::Instant,
    /// Set when the user stops it: whether to send what was recorded.
    send_on_exit: Rc<Cell<Option<bool>>>,
}

impl VoiceBar {
    pub(super) fn new() -> Self {
        let mic = gtk::Button::builder()
            .icon_name("wassup-voice-note-symbolic")
            .tooltip_text(gettext("Record Voice Message"))
            .valign(gtk::Align::End)
            .css_classes(["circular", "suggested-action"])
            .build();
        let cancel = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(gettext("Discard Recording"))
            .css_classes(["circular", "flat"])
            .build();
        let dot = gtk::Label::builder()
            .label("●")
            .css_classes(["recording-dot"])
            .build();
        let time = gtk::Label::builder()
            .label("0:00")
            .hexpand(true)
            .xalign(0.0)
            .css_classes(["numeric"])
            .build();
        let send = gtk::Button::builder()
            .icon_name("wassup-send-symbolic")
            .tooltip_text(gettext("Send Voice Message"))
            .css_classes(["circular", "suggested-action"])
            .build();
        let bar = gtk::Box::builder()
            .spacing(8)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .visible(false)
            .build();
        bar.append(&cancel);
        bar.append(&dot);
        bar.append(&time);
        bar.append(&send);
        Self {
            mic,
            bar,
            time,
            cancel,
            send,
            recording: RefCell::default(),
        }
    }
}

impl Window {
    pub(super) fn connect_voice(self: &Rc<Self>) {
        let v = &self.voice;
        v.mic.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| ui.start_recording()
        ));
        v.cancel.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| ui.stop_recording(false)
        ));
        v.send.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| ui.stop_recording(true)
        ));
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    ui.stop_recording(false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
        v.bar.add_controller(keys);
    }

    /// Starts recording from the default microphone. GStreamer runs in its
    /// own process, so the window's memory does not grow while recording.
    pub(super) fn start_recording(self: &Rc<Self>) {
        if self.voice.recording.borrow().is_some() {
            return;
        }
        self.stop_audio(); // Not to record what is playing.
        let dir = outgoing_dir();
        let path = dir.join(format!("voice-{}.ogg", glib::monotonic_time()));
        let location = format!("location={}", path.display());
        // setpriv makes the kernel kill the recorder if the window dies, so
        // the microphone can never stay open after Wassup, even on a crash.
        let args: Vec<&std::ffi::OsStr> = [
            "setpriv",
            "--pdeathsig",
            "KILL",
            "gst-launch-1.0",
            "-q",
            "-e",
            "autoaudiosrc",
            "!",
            "audioconvert",
            "!",
            "audioresample",
            "!",
            "audio/x-raw,rate=48000,channels=1",
            "!",
            "opusenc",
            "bitrate=24000",
            "!",
            "oggmux",
            "!",
            "filesink",
            &location,
        ]
        .into_iter()
        .map(std::ffi::OsStr::new)
        .collect();
        let spawned = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                gio::Subprocess::newv(
                    &args,
                    gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE,
                )
                .map_err(|e| e.to_string())
            });
        let process = match spawned {
            Ok(process) => process,
            // TRANSLATORS: {} is the reason.
            Err(e) => return self.toast(&gettext("Couldn't record: {}").replace("{}", &e)),
        };
        let send_on_exit = Rc::new(Cell::new(None));
        self.voice.recording.replace(Some(Recording {
            process: process.clone(),
            path: path.clone(),
            started: std::time::Instant::now(),
            send_on_exit: send_on_exit.clone(),
        }));
        self.show_recording_bar(true);
        self.voice.time.set_label("0:00");
        glib::timeout_add_local(
            std::time::Duration::from_millis(500),
            glib::clone!(
                #[weak(rename_to = ui)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    let recording = ui.voice.recording.borrow();
                    let Some(r) = recording.as_ref() else {
                        return glib::ControlFlow::Break;
                    };
                    ui.voice
                        .time
                        .set_label(&clock(r.started.elapsed().as_secs_f64()));
                    glib::ControlFlow::Continue
                }
            ),
        );
        // Ends when the user stops it, or early if there is no microphone.
        let ui = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let _ = process.wait_future().await;
            let Some(ui) = ui.upgrade() else { return };
            let Some(r) = ui.voice.recording.take() else {
                return;
            };
            ui.show_recording_bar(false);
            let long_enough = r.started.elapsed() >= MIN_RECORDING;
            match send_on_exit.get() {
                Some(true) if long_enough && process.is_successful() => {
                    ui.send_file(&r.path.to_string_lossy(), "", false, true);
                }
                Some(_) => {
                    std::fs::remove_file(&r.path).ok();
                }
                None => {
                    std::fs::remove_file(&r.path).ok();
                    ui.toast(&gettext("Couldn't record. Is there a microphone?"));
                }
            }
        });
    }

    /// Stops recording and sends it, or throws it away.
    pub(super) fn stop_recording(&self, send: bool) {
        let recording = self.voice.recording.borrow();
        let Some(r) = recording.as_ref() else { return };
        r.send_on_exit.set(Some(send));
        // SIGINT makes gst-launch -e close the file properly before exiting.
        r.process.send_signal(2);
        let process = r.process.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            if !process.has_exited() {
                process.force_exit();
            }
        });
    }

    fn show_recording_bar(&self, recording: bool) {
        self.voice.bar.set_visible(recording);
        self.composer_bar.set_visible(!recording);
        if recording {
            self.voice.send.grab_focus();
        } else {
            self.composer.grab_focus();
        }
    }
}
