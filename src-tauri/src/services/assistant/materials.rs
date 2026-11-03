//! How a video request uses the files it is given (POST /v1/videos):
//! anchored frames (`frame_images`) or references (`input_references`),
//! never both. One pure decision, shared by the agent tool, the attachment
//! hints and a direct regeneration.
use serde::{Deserialize, Serialize};

pub const MAX_REFERENCE_IMAGES: usize = 9;
pub const MAX_REFERENCE_AUDIO: usize = 3;
pub const MAX_REFERENCE_VIDEO: usize = 1;
pub const MAX_REFERENCE_AUDIO_MS: u64 = 15_000;

pub const FRAMES_IMAGES_ONLY: &str = "assistant_media_frames_images_only";
pub const TOO_MANY_FRAMES: &str = "assistant_media_too_many_frames";
pub const TOO_MANY_REFERENCES: &str = "assistant_media_too_many_references";
pub const AUDIO_TOO_LONG: &str = "assistant_media_reference_audio_too_long";
pub const UNSUPPORTED_MATERIAL: &str = "assistant_media_material_unsupported";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Image,
    Audio,
    Video,
}

impl Kind {
    /// The kind of an attachment by its MIME type; `None` for any file a
    /// video request cannot take.
    pub fn of_mime(mime: &str) -> Option<Self> {
        match mime {
            "image/png" | "image/jpeg" | "image/webp" => Some(Self::Image),
            "audio/mpeg" | "audio/wav" => Some(Self::Audio),
            "video/mp4" => Some(Self::Video),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Material {
    pub id: String,
    pub kind: Kind,
    /// Known for audio; `None` when it could not be read.
    pub duration_ms: Option<u64>,
}

/// What the user asked for. `Auto` follows the API's own reading: one image
/// is the first frame; several images, or any audio or video, are references.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Use {
    #[default]
    Auto,
    /// The first image opens the video; a second one ends it.
    Frames,
    /// Every file guides subject and style.
    Reference,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub first_frame: Option<String>,
    pub last_frame: Option<String>,
    pub references: Vec<Reference>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub id: String,
    pub kind: Kind,
}

impl Plan {
    /// The catalog mode the request needs (`video_attributes.modes`).
    pub fn mode(&self) -> &'static str {
        if self.references.iter().any(|r| r.kind == Kind::Video) {
            "v2v"
        } else if self.first_frame.is_some() || !self.references.is_empty() {
            "i2v"
        } else {
            "t2v"
        }
    }

    pub fn ids(&self) -> Vec<String> {
        self.first_frame
            .iter()
            .chain(self.last_frame.iter())
            .cloned()
            .chain(self.references.iter().map(|r| r.id.clone()))
            .collect()
    }
}

/// The request's frames or references for `materials`, in the user's order.
/// A file named twice counts once.
pub fn plan(materials: &[Material], requested: Use) -> Result<Plan, String> {
    let mut unique: Vec<&Material> = vec![];
    for material in materials {
        if !unique.iter().any(|m| m.id == material.id) {
            unique.push(material);
        }
    }
    let images = unique.iter().filter(|m| m.kind == Kind::Image).count();
    let only_images = images == unique.len();
    let frames = match requested {
        Use::Frames => {
            if !only_images {
                return Err(FRAMES_IMAGES_ONLY.into());
            }
            if images > 2 {
                return Err(TOO_MANY_FRAMES.into());
            }
            true
        }
        Use::Auto => images == 1 && only_images,
        Use::Reference => false,
    };
    if frames {
        return Ok(Plan {
            first_frame: unique.first().map(|m| m.id.clone()),
            last_frame: unique.get(1).map(|m| m.id.clone()),
            references: vec![],
        });
    }
    let count = |kind| unique.iter().filter(|m| m.kind == kind).count();
    if count(Kind::Image) > MAX_REFERENCE_IMAGES
        || count(Kind::Audio) > MAX_REFERENCE_AUDIO
        || count(Kind::Video) > MAX_REFERENCE_VIDEO
    {
        return Err(TOO_MANY_REFERENCES.into());
    }
    if unique.iter().any(|m| {
        m.kind == Kind::Audio && m.duration_ms.is_none_or(|ms| ms > MAX_REFERENCE_AUDIO_MS)
    }) {
        return Err(AUDIO_TOO_LONG.into());
    }
    Ok(Plan {
        first_frame: None,
        last_frame: None,
        references: unique
            .iter()
            .map(|m| Reference {
                id: m.id.clone(),
                kind: m.kind,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(id: &str) -> Material {
        Material {
            id: id.into(),
            kind: Kind::Image,
            duration_ms: None,
        }
    }
    fn audio(id: &str, ms: u64) -> Material {
        Material {
            id: id.into(),
            kind: Kind::Audio,
            duration_ms: Some(ms),
        }
    }
    fn video(id: &str) -> Material {
        Material {
            id: id.into(),
            kind: Kind::Video,
            duration_ms: None,
        }
    }
    fn refs(plan: &Plan) -> Vec<&str> {
        plan.references.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn no_files_is_text_to_video() {
        let plan = plan(&[], Use::Auto).unwrap();
        assert_eq!(plan, Plan::default());
        assert_eq!(plan.mode(), "t2v");
    }

    #[test]
    fn one_image_is_the_first_frame_by_default() {
        let plan = plan(&[image("a")], Use::Auto).unwrap();
        assert_eq!(plan.first_frame.as_deref(), Some("a"));
        assert!(plan.last_frame.is_none() && plan.references.is_empty());
        assert_eq!(plan.mode(), "i2v");
    }

    #[test]
    fn several_images_are_references_by_default() {
        let plan = plan(&[image("a"), image("b")], Use::Auto).unwrap();
        assert!(plan.first_frame.is_none());
        assert_eq!(refs(&plan), ["a", "b"]);
        assert_eq!(plan.mode(), "i2v");
    }

    #[test]
    fn an_image_with_audio_or_video_makes_every_file_a_reference() {
        let with_audio = plan(&[image("a"), audio("v", 9_000)], Use::Auto).unwrap();
        assert_eq!(refs(&with_audio), ["a", "v"]);
        assert!(with_audio.first_frame.is_none());
        let with_video = plan(&[image("a"), video("m")], Use::Auto).unwrap();
        assert_eq!(refs(&with_video), ["a", "m"]);
        assert_eq!(with_video.mode(), "v2v");
    }

    #[test]
    fn asked_frames_use_the_first_image_to_open_and_the_second_to_end() {
        let plan = plan(&[image("start"), image("end")], Use::Frames).unwrap();
        assert_eq!(plan.first_frame.as_deref(), Some("start"));
        assert_eq!(plan.last_frame.as_deref(), Some("end"));
        assert!(plan.references.is_empty());
    }

    #[test]
    fn frames_take_only_up_to_two_images() {
        assert_eq!(
            plan(&[image("a"), image("b"), image("c")], Use::Frames).unwrap_err(),
            TOO_MANY_FRAMES
        );
        assert_eq!(
            plan(&[image("a"), audio("v", 1_000)], Use::Frames).unwrap_err(),
            FRAMES_IMAGES_ONLY
        );
    }

    #[test]
    fn frames_and_references_never_mix() {
        for requested in [Use::Auto, Use::Frames, Use::Reference] {
            for files in [
                vec![image("a")],
                vec![image("a"), image("b")],
                vec![image("a"), audio("v", 1_000)],
            ] {
                if let Ok(plan) = plan(&files, requested) {
                    let frames = plan.first_frame.is_some() || plan.last_frame.is_some();
                    assert!(!frames || plan.references.is_empty());
                }
            }
        }
    }

    #[test]
    fn a_single_image_asked_as_reference_stays_a_reference() {
        let plan = plan(&[image("a")], Use::Reference).unwrap();
        assert!(plan.first_frame.is_none());
        assert_eq!(refs(&plan), ["a"]);
    }

    #[test]
    fn reference_limits_follow_the_api() {
        let nine: Vec<_> = (0..9).map(|i| image(&format!("i{i}"))).collect();
        assert_eq!(plan(&nine, Use::Auto).unwrap().references.len(), 9);
        let ten: Vec<_> = (0..10).map(|i| image(&format!("i{i}"))).collect();
        assert_eq!(plan(&ten, Use::Auto).unwrap_err(), TOO_MANY_REFERENCES);
        let four_audio: Vec<_> = (0..4).map(|i| audio(&format!("a{i}"), 1_000)).collect();
        assert_eq!(
            plan(&four_audio, Use::Auto).unwrap_err(),
            TOO_MANY_REFERENCES
        );
        assert_eq!(
            plan(&[video("a"), video("b")], Use::Auto).unwrap_err(),
            TOO_MANY_REFERENCES
        );
    }

    #[test]
    fn reference_audio_must_be_known_and_at_most_fifteen_seconds() {
        assert!(plan(&[audio("a", 15_000)], Use::Auto).is_ok());
        assert_eq!(
            plan(&[audio("a", 15_001)], Use::Auto).unwrap_err(),
            AUDIO_TOO_LONG
        );
        let unknown = Material {
            duration_ms: None,
            ..audio("a", 0)
        };
        assert_eq!(plan(&[unknown], Use::Auto).unwrap_err(), AUDIO_TOO_LONG);
    }

    #[test]
    fn a_file_named_twice_counts_once() {
        let plan = plan(&[image("a"), image("a")], Use::Auto).unwrap();
        assert_eq!(plan.first_frame.as_deref(), Some("a"));
    }

    #[test]
    fn kinds_come_from_the_attachment_mime_type() {
        assert_eq!(Kind::of_mime("image/webp"), Some(Kind::Image));
        assert_eq!(Kind::of_mime("audio/mpeg"), Some(Kind::Audio));
        assert_eq!(Kind::of_mime("video/mp4"), Some(Kind::Video));
        assert_eq!(Kind::of_mime("image/gif"), None);
        assert_eq!(Kind::of_mime("application/pdf"), None);
    }
}
