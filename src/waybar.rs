use crate::utils::Command;

#[cfg(feature = "mpd")]
use crate::mpd::MpdFormatter;
#[cfg(feature = "mpd")]
use crate::text_source::TextSource;

use super::RunningText;

#[derive(Debug)]
pub enum Tooltip {
    Simple(String),
    Cmd(Command),
    #[cfg(feature = "mpd")]
    Mpd(MpdFormatter),
}
pub struct RunningTextWithTooltip {
    text: RunningText,
    tooltip: Tooltip,
    buffer: String,
}

impl RunningTextWithTooltip {
    pub fn new(text: RunningText, tooltip: Tooltip) -> RunningTextWithTooltip {
        RunningTextWithTooltip {
            text,
            tooltip,
            buffer: String::new(),
        }
    }
}

impl Iterator for RunningTextWithTooltip {
    type Item = (anyhow::Result<String>, String);

    fn next(&mut self) -> Option<Self::Item> {
        let iteration = self.text.next().unwrap();
        let src = self.text.get_source();
        let tooltip = match (&mut self.tooltip, src) {
            (Tooltip::Simple(s), _) => s,
            (Tooltip::Cmd(cmd), _) => {
                cmd.spawn_and_read_output()
                    .expect("Child error")
                    .clone_into(&mut self.buffer);
                self.buffer.retain(|c| c != '\n');
                &self.buffer
            }
            #[cfg(feature = "mpd")]
            (Tooltip::Mpd(f), TextSource::Mpd(s)) => {
                self.buffer.clear();
                f.format_with_source(s, &mut self.buffer)
                    .expect("MPD format error");
                self.buffer.retain(|c| c != '\n');
                &self.buffer
            }
            #[cfg(feature = "mpd")]
            (Tooltip::Mpd(_), _) => panic!("MPD format for tooltip can only be used with --mpd"),
        };
        Some((iteration, tooltip.to_owned()))
    }
}
