//! Enums with Android's kotlinx `@SerialName`s.
//!
//! Serialization writes the lowercase serial name. Deserialization also accepts any ASCII case
//! (legacy desktop wrote PascalCase sources such as `"Spotify"`). `name()` is the Kotlin enum
//! constant name (`toString()`), e.g. `SPOTIFY`, used by signatures and config-file ids.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! serial_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident { $($(#[$vmeta:meta])* $variant:ident => ($serial:literal, $kotlin:literal)),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name { $($(#[$vmeta])* $variant),+ }

        impl $name {
            /// Every variant in Kotlin declaration order.
            pub const ALL_VARIANTS: &'static [Self] = &[$(Self::$variant),+];

            /// The kotlinx serial name (lowercase).
            #[must_use]
            pub const fn serial_name(self) -> &'static str {
                match self { $(Self::$variant => $serial),+ }
            }

            /// The Kotlin enum constant name (`toString()`), uppercase.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $kotlin),+ }
            }

            /// Parses a serial or constant name, ignoring ASCII case.
            #[must_use]
            pub fn parse_lenient(value: &str) -> Option<Self> {
                $(if value.eq_ignore_ascii_case($serial) { return Some(Self::$variant); })+
                None
            }
        }

        impl fmt::Display for $name {
            /// Kotlin `toString()`: the uppercase constant name.
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.name())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.serial_name())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct V;
                impl Visitor<'_> for V {
                    type Value = $name;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str(concat!("a ", stringify!($name), " name"))
                    }
                    fn visit_str<E: de::Error>(self, v: &str) -> Result<$name, E> {
                        $name::parse_lenient(v).ok_or_else(|| {
                            de::Error::unknown_variant(v, &[$($serial),+])
                        })
                    }
                }
                deserializer.deserialize_str(V)
            }
        }
    };
}

serial_enum! {
    /// Track/playlist provider.
    pub enum SourceType {
        Spotify => ("spotify", "SPOTIFY"),
        Jellyfin => ("jellyfin", "JELLYFIN"),
        Plex => ("plex", "PLEX"),
        Custom => ("custom", "CUSTOM"),
        All => ("all", "ALL"),
    }
}

serial_enum! {
    /// Custom playlist kind.
    pub enum PlaylistType {
        Standard => ("standard", "STANDARD"),
        Union => ("union", "UNION"),
    }
}

serial_enum! {
    #[derive(Default)]
    pub enum RepeatMode {
        #[default]
        Off => ("off", "OFF"),
        One => ("one", "ONE"),
        All => ("all", "ALL"),
    }
}

serial_enum! {
    /// Transport state carried in the legacy `PlaybackStatus` wire shape.
    pub enum PlaybackStateType {
        Idle => ("idle", "IDLE"),
        Playing => ("playing", "PLAYING"),
        Paused => ("paused", "PAUSED"),
        Buffering => ("buffering", "BUFFERING"),
        Error => ("error", "ERROR"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_use_lowercase_serial_names() {
        assert_eq!(serde_json::to_string(&PlaylistType::Union).unwrap(), "\"union\"");
        assert_eq!(serde_json::to_string(&RepeatMode::All).unwrap(), "\"all\"");
        assert_eq!(
            serde_json::to_string(&PlaybackStateType::Buffering).unwrap(),
            "\"buffering\""
        );
        assert_eq!(SourceType::Spotify.to_string(), "SPOTIFY");
    }

    #[test]
    fn sources_decode_in_any_case() {
        for raw in ["\"spotify\"", "\"Spotify\"", "\"SPOTIFY\""] {
            assert_eq!(serde_json::from_str::<SourceType>(raw).unwrap(), SourceType::Spotify);
        }
        assert!(serde_json::from_str::<SourceType>("\"tidal\"").is_err());
    }
}
