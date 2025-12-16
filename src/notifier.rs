use std::sync::mpsc::{channel, Sender};
use std::thread;

pub struct Notifier {
    sender: Sender<(String, String)>,
}

impl Notifier {
    pub fn new() -> Self {
        let (sender, receiver) = channel::<(String, String)>();

        thread::spawn(move || {
            while let Ok((title, body)) = receiver.recv() {
                Self::send_notification(&title, &body);
            }
        });

        Notifier { sender }
    }

    pub fn notify(&self, title: &str, body: &str) {
        let _ = self.sender.send((title.to_string(), body.to_string()));
    }

    fn send_notification(title: &str, body: &str) {
        match std::process::Command::new("notify-send")
            .arg("-a")
            .arg("Whisper Tool")
            .arg("-i")
            .arg("audio-input-microphone")
            .arg("-t")
            .arg("5000")
            .arg(title)
            .arg(body)
            .spawn()
        {
            Ok(_) => println!("✓ Notification: {} - {}", title, body),
            Err(e) => eprintln!("✗ Notification failed: {}", e),
        }
    }
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new()
    }
}
