use std::{
    collections::HashMap,
    error::Error,
    fmt::Display,
    fmt::{self, Write},
    net::SocketAddr,
    num::ParseIntError,
    str::FromStr,
    time::Duration,
};

use anyhow::Context;
use chrono::{
    format::{Item, StrftimeItems},
    NaiveTime,
};
use mpd::{song::QueuePlace, Client, Song, State, Status};

use crate::text_source::ContentChange;

#[derive(Debug)]
pub enum IconSetParseError<const N: usize> {
    NotEnoughChars,
    TooManyChars,
}
impl<const N: usize> Display for IconSetParseError<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IconSetParseError::NotEnoughChars => {
                write!(f, "Not enough characters (expected {})", N)
            }
            IconSetParseError::TooManyChars => write!(f, "Too many characters (expected {})", N),
        }
    }
}
impl<const N: usize> Error for IconSetParseError<N> {}

#[derive(Debug, Clone)]
pub struct StateStatusIcons {
    play: char,
    pause: char,
    stop: char,
}

impl StateStatusIcons {
    pub fn get_icon(&self, state: State) -> char {
        match state {
            State::Stop => self.stop,
            State::Play => self.play,
            State::Pause => self.pause,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatusIcons {
    enabled: char,
    disabled: Option<char>,
}

impl StatusIcons {
    pub fn get_icon(&self, state: bool) -> Option<char> {
        if state {
            Some(self.enabled)
        } else {
            self.disabled
        }
    }

    pub fn write<T: Write>(&self, state: bool, pad: usize, f: &mut T) -> std::fmt::Result {
        if let Some(c) = self.get_icon(state) {
            write!(f, "{}{}", c, " ".repeat(pad))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
pub struct StatusIconsSet {
    state: StateStatusIcons,
    consume: StatusIcons,
    random: StatusIcons,
    repeat: StatusIcons,
    single: StatusIcons,
}

impl StatusIconsSet {
    pub fn new(
        state_icons: StateStatusIcons,
        consume_icons: StatusIcons,
        random_icons: StatusIcons,
        repeat_icons: StatusIcons,
        single_icons: StatusIcons,
    ) -> Self {
        Self {
            state: state_icons,
            consume: consume_icons,
            random: random_icons,
            repeat: repeat_icons,
            single: single_icons,
        }
    }

    pub fn write_bool<T: Write>(&self, ph: &Placeholder, value: bool, f: &mut T) -> fmt::Result {
        match ph {
            Placeholder::ConsumeIcon(pad) => self.consume.write(value, *pad, f),
            Placeholder::RandomIcon(pad) => self.random.write(value, *pad, f),
            Placeholder::RepeatIcon(pad) => self.repeat.write(value, *pad, f),
            Placeholder::SingleIcon(pad) => self.single.write(value, *pad, f),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Placeholder {
    String(String),
    Artist,
    AlbumArtist,
    Album,
    Title,
    Filename,
    Date,
    TotalTime(Vec<Item<'static>>),
    ElapsedTime(Vec<Item<'static>>),
    Volume,
    SongPosition,
    QueueLength,
    StateIcon(usize),
    ConsumeIcon(usize),
    RandomIcon(usize),
    RepeatIcon(usize),
    SingleIcon(usize),
}

#[derive(Debug, PartialEq)]
pub enum PlaceholderValue<'a> {
    String(&'a str),
    OptionalString(Option<&'a str>),
    Volume(i8),
    OptionalDuration(Option<Duration>, &'a Vec<Item<'static>>),
    OptionalQueuePlace(Option<QueuePlace>),
    Len(u32),
    Bool(bool),
    State(State, usize),
}

impl Placeholder {
    pub fn get<'a>(&'a self, song: Option<&'a Song>, status: &Status) -> PlaceholderValue<'a> {
        let mut tags: HashMap<&str, &str> = song
            .map(|s| {
                s.tags
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        match self {
            Placeholder::String(s) => PlaceholderValue::String(s),
            Placeholder::Artist => PlaceholderValue::OptionalString(
                song.map(|s| s.artist.as_deref()).unwrap_or_default(),
            ),
            Placeholder::AlbumArtist => {
                PlaceholderValue::OptionalString(tags.remove("AlbumArtist"))
            }
            Placeholder::Album => PlaceholderValue::OptionalString(tags.remove("Album")),
            Placeholder::Title => PlaceholderValue::OptionalString(
                song.map(|s| s.title.as_deref()).unwrap_or_default(),
            ),
            Placeholder::Filename => {
                PlaceholderValue::OptionalString(song.map(|s| s.file.as_str()))
            }
            Placeholder::Date => PlaceholderValue::OptionalString(tags.remove("Date")),
            Placeholder::Volume => PlaceholderValue::Volume(status.volume),
            Placeholder::ElapsedTime(fmt) => {
                PlaceholderValue::OptionalDuration(status.elapsed, fmt)
            }
            Placeholder::TotalTime(fmt) => PlaceholderValue::OptionalDuration(status.duration, fmt),
            Placeholder::SongPosition => PlaceholderValue::OptionalQueuePlace(status.song),
            Placeholder::QueueLength => PlaceholderValue::Len(status.queue_len),
            Placeholder::StateIcon(pad) => PlaceholderValue::State(status.state, *pad),
            Placeholder::ConsumeIcon(_) => PlaceholderValue::Bool(status.consume),
            Placeholder::RandomIcon(_) => PlaceholderValue::Bool(status.random),
            Placeholder::RepeatIcon(_) => PlaceholderValue::Bool(status.repeat),
            Placeholder::SingleIcon(_) => PlaceholderValue::Bool(status.single),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MpdFormatter(Vec<Placeholder>);

#[derive(Debug)]
pub enum MpdFormatParseError {
    UnknownPlaceholder(String),
    RedundantFormat(String),
    DurationParseError(chrono::format::ParseError),
    PadParseError(ParseIntError),
    UnmatchedParenthesis,
}

impl Display for MpdFormatParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPlaceholder(placeholder) => {
                write!(f, "Unknown placeholder '{placeholder}'")
            }
            Self::RedundantFormat(placeholder) => {
                write!(f, "'{placeholder}' does not have additional formatting")
            }
            Self::DurationParseError(e) => {
                write!(f, "Invalid duration format: {e}")
            }
            Self::PadParseError(e) => write!(f, "Padding parse error: {e}"),
            Self::UnmatchedParenthesis => write!(f, "Unmatched '{{' or '}}"),
        }
    }
}
impl Error for MpdFormatParseError {}

#[derive(Debug)]
pub struct MpdSource {
    client: Client,
    current_song: Option<Song>,
    current_status: Status,
    running_format: MpdFormatter,
    prefix_format: MpdFormatter,
    suffix_format: MpdFormatter,
    icons: StatusIconsSet,
    default_placeholder: String,
}

impl MpdSource {
    pub fn new(
        addr: SocketAddr,
        fmt: MpdFormatter,
        prefix: MpdFormatter,
        suffix: MpdFormatter,
        icons: StatusIconsSet,
        default_placeholder: String,
    ) -> anyhow::Result<Self> {
        let mut client = Client::connect(addr).context("MPD connection error")?;
        Ok(Self {
            current_song: client.currentsong().context("MPD server error")?,
            current_status: client.status().context("MPD server error")?,
            client,
            running_format: fmt,
            prefix_format: prefix,
            suffix_format: suffix,
            icons,
            default_placeholder,
        })
    }
    pub fn get(
        &mut self,
        content: &mut String,
        prefix: &mut String,
        suffix: &mut String,
    ) -> anyhow::Result<ContentChange> {
        let song = self.client.currentsong().context("MPD server error")?;
        let status = self.client.status().context("MPD server error")?;
        let mut change = ContentChange::empty();
        // I made this because I think this looks hilarious and I don't want to repeat this
        macro_rules! change {
            {
                $($var:ident if $type:ident in $fmt:ident;)*
            } => {
                $(
                    change.set(
                        ContentChange::$type,
                        self.$fmt
                        .iter()
                        .any(|ph| ph.get(self.current_song(), self.current_status()) != ph.get(song.as_ref(), &status)),
                    );
                )*
                $(
                    if change.contains(ContentChange::$type) {
                        $var.clear();
                        self.$fmt.format(
                            &self.icons,
                            song.as_ref(),
                            &status,
                            &self.default_placeholder,
                            $var,
                        )?;
                    }
                )*
            };
        }
        change! {
            prefix if Prefix in prefix_format;
            suffix if Suffix in suffix_format;
            content if Running in running_format;
        }
        (self.current_song, self.current_status) = (song, status);
        Ok(change)
    }
    pub fn running_format(&self) -> &MpdFormatter {
        &self.running_format
    }
    pub fn prefix_format(&self) -> &MpdFormatter {
        &self.prefix_format
    }
    pub fn suffix_format(&self) -> &MpdFormatter {
        &self.suffix_format
    }
    pub fn icons(&self) -> &StatusIconsSet {
        &self.icons
    }
    pub fn current_song(&self) -> Option<&Song> {
        self.current_song.as_ref()
    }
    pub fn current_status(&self) -> &Status {
        &self.current_status
    }
}

impl MpdFormatter {
    pub fn only_string(str: String) -> Self {
        Self(vec![Placeholder::String(str)])
    }
    pub fn format_with_source(&self, source: &MpdSource, f: &mut String) -> anyhow::Result<()> {
        self.format(
            source.icons(),
            source.current_song(),
            source.current_status(),
            &source.default_placeholder,
            f,
        )
    }
    pub fn format(
        &self,
        icons: &StatusIconsSet,
        song: Option<&Song>,
        status: &Status,
        default: &str,
        f: &mut String,
    ) -> anyhow::Result<()> {
        for ph in self.iter() {
            match ph.get(song, status) {
                PlaceholderValue::String(s) => write!(f, "{}", s)?,
                PlaceholderValue::OptionalString(s) => write!(f, "{}", s.unwrap_or(default))?,
                PlaceholderValue::Volume(v) => write!(f, "{}", v)?,
                PlaceholderValue::Len(l) => write!(f, "{}", l)?,
                PlaceholderValue::OptionalDuration(op, fmt) => match op {
                    Some(d) => write!(
                        f,
                        "{}",
                        chrono::format::DelayedFormat::new(
                            None,
                            NaiveTime::from_num_seconds_from_midnight_opt(
                                d.as_secs() as _,
                                d.subsec_nanos() as _
                            ),
                            fmt.iter()
                        )
                    )
                    .map_err(|e| anyhow::anyhow!(e).context("Unsupported time specifier"))?,
                    None => write!(f, "{}", default)?,
                },
                PlaceholderValue::OptionalQueuePlace(op) => match op {
                    Some(qp) => write!(f, "{}", qp.id),
                    None => write!(f, "{}", default),
                }?,
                PlaceholderValue::Bool(b) => icons.write_bool(ph, b, f)?,
                PlaceholderValue::State(s, pad) => {
                    write!(f, "{}{}", icons.state.get_icon(s), " ".repeat(pad))?
                }
            };
        }
        Ok(())
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Placeholder> {
        self.0.iter()
    }
}

impl Display for MpdFormatter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for ph in self.iter() {
            if let Placeholder::String(s) = ph {
                for part in s.split_inclusive(['{', '}']) {
                    write!(f, "{}", part)?;
                    match part.chars().last().expect("Part must not be empty") {
                        c if matches!(c, '{' | '}') => write!(f, "{}", c)?,
                        _ => continue,
                    };
                }
            } else {
                write!(
                    f,
                    "{}",
                    match ph {
                        Placeholder::Album => "{album}",
                        Placeholder::AlbumArtist => "{albumArtist}",
                        Placeholder::Artist => "{artist}",
                        Placeholder::ConsumeIcon(_) => "{consumeIcon}",
                        Placeholder::Date => "{date}",
                        Placeholder::ElapsedTime(_) => "{elapsedTime}",
                        Placeholder::Filename => "{filename}",
                        Placeholder::QueueLength => "{queueLength}",
                        Placeholder::RandomIcon(_) => "{randomIcon}",
                        Placeholder::RepeatIcon(_) => "{repeatIcon}",
                        Placeholder::SingleIcon(_) => "{singleIcon}",
                        Placeholder::SongPosition => "{songPosition}",
                        Placeholder::StateIcon(_) => "{stateIcon}",
                        Placeholder::Title => "{title}",
                        Placeholder::TotalTime(_) => "{totalTime}",
                        Placeholder::Volume => "{volume}",
                        Placeholder::String(_) => unreachable!(),
                    }
                )?;
            }
        }
        Ok(())
    }
}

impl FromStr for MpdFormatter {
    type Err = MpdFormatParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut placeholders = Vec::new();
        let mut raw = String::new();
        let mut parse_slice = s;
        while !parse_slice.is_empty() {
            let left_par = match parse_slice.find(['{', '}']) {
                Some(i) => i,
                None => {
                    raw.push_str(parse_slice);
                    break;
                }
            };
            if let Some('}') = &parse_slice[left_par..].chars().next() {
                match parse_slice[left_par + 1..].chars().next() {
                    Some('}') => {
                        raw.push_str(&parse_slice[..left_par + 1]);
                        parse_slice = &parse_slice[left_par + 2..];
                        continue;
                    }
                    _ => return Err(MpdFormatParseError::UnmatchedParenthesis),
                };
            }

            if let Some('{') = &parse_slice[left_par + 1..].chars().next() {
                raw.push_str(&parse_slice[..left_par + 1]);
                parse_slice = &parse_slice[left_par + 2..];
                continue;
            }
            raw.push_str(&parse_slice[..left_par]);
            parse_slice = &parse_slice[left_par + 1..];
            if !raw.is_empty() {
                placeholders.push(Placeholder::String(raw));
                raw = String::new();
            }

            let right_par = match parse_slice.find(['{', '}']) {
                Some(i) => i,
                None => return Err(MpdFormatParseError::UnmatchedParenthesis),
            };
            if let Some('{') = parse_slice[right_par..].chars().next() {
                return Err(MpdFormatParseError::UnmatchedParenthesis);
            }
            let ph_spec = &parse_slice[..right_par];
            placeholders.push(if let Some((ph_type, ph_fmt)) = ph_spec.split_once(':') {
                match ph_type {
                    "date" => Placeholder::Date,
                    "elapsedTime" => Placeholder::ElapsedTime(
                        StrftimeItems::new(ph_fmt)
                            .parse_to_owned()
                            .map_err(MpdFormatParseError::DurationParseError)?,
                    ),
                    "totalTime" => Placeholder::TotalTime(
                        StrftimeItems::new(ph_fmt)
                            .parse_to_owned()
                            .map_err(MpdFormatParseError::DurationParseError)?,
                    ),
                    "consumeIcon" | "repeatIcon" | "stateIcon" | "singleIcon" | "randomIcon" => {
                        let pad = ph_fmt
                            .parse::<usize>()
                            .map_err(MpdFormatParseError::PadParseError)?;
                        match ph_type {
                            "consumeIcon" => Placeholder::ConsumeIcon(pad),
                            "repeatIcon" => Placeholder::RepeatIcon(pad),
                            "stateIcon" => Placeholder::StateIcon(pad),
                            "singleIcon" => Placeholder::SingleIcon(pad),
                            "randomIcon" => Placeholder::RandomIcon(pad),
                            _ => unreachable!(),
                        }
                    }
                    _ => return Err(MpdFormatParseError::RedundantFormat(ph_type.to_owned())),
                }
            } else {
                match ph_spec {
                    "album" => Placeholder::Album,
                    "albumArtist" => Placeholder::AlbumArtist,
                    "artist" => Placeholder::Artist,
                    "consumeIcon" => Placeholder::ConsumeIcon(0),
                    "date" => Placeholder::Date,
                    "elapsedTime" => Placeholder::ElapsedTime(
                        StrftimeItems::new("%M:%S").parse_to_owned().unwrap(),
                    ),
                    "filename" => Placeholder::Filename,
                    "queueLength" => Placeholder::QueueLength,
                    "randomIcon" => Placeholder::RandomIcon(0),
                    "repeatIcon" => Placeholder::RepeatIcon(0),
                    "singleIcon" => Placeholder::SingleIcon(0),
                    "songPosition" => Placeholder::SongPosition,
                    "stateIcon" => Placeholder::StateIcon(0),
                    "title" => Placeholder::Title,
                    "totalTime" => Placeholder::TotalTime(
                        StrftimeItems::new("%M:%S").parse_to_owned().unwrap(),
                    ),
                    "volume" => Placeholder::Volume,
                    _ => {
                        return Err(MpdFormatParseError::UnknownPlaceholder(
                            parse_slice[..right_par].to_owned(),
                        ))
                    }
                }
            });
            parse_slice = &parse_slice[right_par + 1..];
        }
        if !raw.is_empty() {
            placeholders.push(Placeholder::String(raw));
        }
        Ok(Self(placeholders))
    }
}

macro_rules! next_or_err {
    ($iter:ident => $type:ident: $($field:ident),+) => {
        $type {
            $($field: $iter.next().ok_or(IconSetParseError::NotEnoughChars)?),+
        }
    };
}

impl FromStr for StateStatusIcons {
    type Err = IconSetParseError<3>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut iter = s.chars();
        let result = Ok(next_or_err!(iter => StateStatusIcons: play, pause, stop));
        if iter.next().is_some() {
            return Err(IconSetParseError::TooManyChars);
        }
        result
    }
}

impl FromStr for StatusIcons {
    type Err = IconSetParseError<2>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut iter = s.chars();
        let result = Ok(StatusIcons {
            enabled: iter.next().ok_or(IconSetParseError::NotEnoughChars)?,
            disabled: iter.next(),
        });
        if iter.next().is_some() {
            return Err(IconSetParseError::TooManyChars);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::mpd::{MpdFormatParseError, MpdFormatter, Placeholder};
    use chrono::format::StrftimeItems;
    macro_rules! ph {
        ($p:ident) => {
            Placeholder::$p
        };
        ($p:ident(#$v:literal)) => {
            Placeholder::$p($v)
        };
        ($p:ident(*$v:literal)) => {
            Placeholder::$p(StrftimeItems::new($v).parse_to_owned().unwrap())
        };
        ($str:literal) => {
            Placeholder::String($str.to_owned())
        };
    }
    #[test]
    fn format_parse_test() {
        macro_rules! assert_ok {
            ($str:literal => [$($item:tt$(($h:tt$time:literal))?),*]) => {
                assert_eq!($str.parse::<MpdFormatter>().unwrap().0, vec![$(ph!($item$(($h$time))?)),*])
            };
        }
        macro_rules! assert_err {
            ($str:literal => $err:ident$(($s:literal))?) => {
                assert!(matches!($str.parse::<MpdFormatter>().unwrap_err(), MpdFormatParseError::$err$((s) if s.as_str() == $s)?));
            };
        }
        assert_ok!("rawstr" => ["rawstr"]);
        assert_ok!("" => []);
        assert_ok!("{artist} - {title}" => [Artist, " - ", Title]);
        assert_ok!(" [{elapsedTime}/{totalTime}] {stateIcon}" => [" [", ElapsedTime(*"%M:%S"), "/", TotalTime(*"%M:%S"), "] ", StateIcon(#0)]);
        assert_ok!(
            " [{elapsedTime:%M with %S}/{totalTime:%H hours %M minutes %S seconds}] {stateIcon:1}"
            => [" [", ElapsedTime(*"%M with %S"), "/", TotalTime(*"%H hours %M minutes %S seconds"), "] ", StateIcon(#1)]
        );
        assert_ok!("{{}}" => ["{}"]);
        assert_ok!("{{{artist}}}" => ["{", Artist, "}"]);
        assert_ok!("{{{artist}{title}}}" => ["{", Artist, Title, "}"]);
        assert_ok!("{artist} {title}}}" => [Artist, " ", Title, "}"]);
        assert_ok!("}}{{}}}}" => ["}{}}"]);
        assert_ok!("{{{artist}}}{title}" => ["{", Artist, "}", Title]);
        assert_ok!("{artist}{title}" => ["", Artist, "", Title, ""]);
        assert_ok!("}}{{{artist}}}{title}}}" => ["}{", Artist, "}", Title, "}"]);
        assert_err!("{artst}" => UnknownPlaceholder("artst"));
        assert_err!("{}artist}}" => UnknownPlaceholder(""));
        assert_err!("{ar}tst}" => UnknownPlaceholder("ar"));
        assert_err!("{artist}}" => UnmatchedParenthesis);
        assert_err!("}{artist}" => UnmatchedParenthesis);
        assert_err!("}}{{{artist}}}{title}}}" => UnknownPlaceholder("artist"));
        assert_err!("}}{{{artist}}}{{title}}}" => UnmatchedParenthesis);
        assert_err!("{{{{artist}}}" => UnmatchedParenthesis);
        assert_err!("{{{artist}}}{" => UnmatchedParenthesis);
        assert_err!("{{{artist}}}}" => UnmatchedParenthesis);
    }

    #[test]
    fn format_display_test() {
        macro_rules! assert {
            ([$($item:tt),*] => $str:literal) => {
                assert_eq!(MpdFormatter(vec![$(ph!($item)),*]).to_string(), $str)
            };
        }
        assert!([Artist, " - ", Title] => "{artist} - {title}");
        assert!([Artist, "{ - }", Title] => "{artist}{{ - }}{title}");
        assert!(["}", Artist, "{ -{ }", Title] => "}}{artist}{{ -{{ }}{title}");
        assert!([] => "");
    }

    #[test]
    fn format_back_and_forth_test() {
        macro_rules! assert {
            ($str:literal) => {
                assert_eq!($str.parse::<MpdFormatter>().unwrap().to_string(), $str)
            };
        }
        assert!("rawstr");
        assert!("");
        assert!("{artist} - {title}");
        assert!("{{}}");
        assert!("{{{artist}}}");
        assert!("{{{artist}{title}}}");
        assert!("{artist} {title}}}");
        assert!("}}{{}}}}");
        assert!("{{{artist}}}{title}");
        assert!("{artist}{title}");
        assert!("}}{{{artist}}}{title}}}");
    }
}
