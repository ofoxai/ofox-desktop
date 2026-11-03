//! Generation tools a chat model calls: images, videos and speech, made by
//! the host with the conversation's media defaults. What the user said in the
//! conversation (the tool's arguments) wins over a default; a default the
//! chosen model does not offer is left out rather than refused.
pub(super) mod host;
pub use host::RegenerateRequest;

use super::{
    catalog::{Kind, Model},
    materials,
    media::ImageRequest,
    video::VideoRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const IMAGE: &str = "generate_image";
pub const VIDEO: &str = "generate_video";
pub const SPEECH: &str = "generate_speech";

/// The most images one run may make, across all of its calls.
pub const MAX_IMAGES_PER_RUN: usize = 4;
/// The most speech files one run may make.
pub const MAX_SPEECH_PER_RUN: usize = 3;
pub const SPEECH_LIMIT: &str = "assistant_media_speech_turn_limit";
/// The longest prompt a generation sends, in characters.
pub const MAX_PROMPT_CHARS: usize = 4_000;

pub const INVALID: &str = "assistant_media_invalid_params";
pub const NO_MODEL: &str = "assistant_media_model_unavailable";
pub const TURN_LIMIT: &str = "assistant_media_image_turn_limit";
pub const DECLINED: &str = "assistant_media_declined";

pub fn is_media_tool(name: &str) -> bool {
    matches!(name, IMAGE | VIDEO | SPEECH)
}

/// The generation defaults of one conversation, set under the composer. A
/// new conversation starts from the account's last choice.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageDefaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoDefaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech: Option<SpeechDefaults>,
    /// The generation chips shown beside "+": `image`, `video`, `speech`.
    /// They tell the model what the user is about to ask for; the tools are
    /// there without them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageDefaults {
    pub model: Option<String>,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub n: Option<u8>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoDefaults {
    pub model: Option<String>,
    pub duration: Option<u64>,
    pub resolution: Option<String>,
    pub aspect_ratio: Option<String>,
    pub generate_audio: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeechDefaults {
    pub model: Option<String>,
    pub voice: Option<String>,
    pub format: Option<String>,
    pub language: Option<String>,
}

impl MediaDefaults {
    /// Bounded strings and known chips only; models are checked when used.
    pub fn validate(&self) -> Result<(), String> {
        let short = |value: &Option<String>| value.as_ref().is_none_or(|s| s.len() <= 256);
        let image = self.image.as_ref().is_none_or(|d| {
            short(&d.model) && short(&d.size) && short(&d.quality) && d.n.is_none_or(|n| n >= 1)
        });
        let video = self
            .video
            .as_ref()
            .is_none_or(|d| short(&d.model) && short(&d.resolution) && short(&d.aspect_ratio));
        let speech = self.speech.as_ref().is_none_or(|d| {
            short(&d.model) && short(&d.voice) && short(&d.format) && short(&d.language)
        });
        let hints = self.hints.len() <= 3
            && self
                .hints
                .iter()
                .all(|h| matches!(h.as_str(), "image" | "video" | "speech"));
        if image && video && speech && hints {
            Ok(())
        } else {
            Err(INVALID.into())
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageArgs {
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub n: Option<u8>,
    /// An attachment to change instead of drawing from nothing.
    #[serde(default)]
    pub image_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VideoArgs {
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub duration: Option<u64>,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub aspect_ratio: Option<String>,
    #[serde(default)]
    pub generate_audio: Option<bool>,
    /// Images, audio and video to start from, in the user's order.
    #[serde(default)]
    pub material_ids: Vec<String>,
    #[serde(default)]
    pub material_use: materials::Use,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpeechArgs {
    pub text: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    /// How to speak, for a model that takes instructions.
    #[serde(default)]
    pub style: Option<String>,
}

/// A refusal the model can act on: `<code>: <what to change>`.
fn refuse(code: &str, why: impl AsRef<str>) -> String {
    format!("{code}: {}", why.as_ref())
}

fn allowed(name: &str, values: &[String]) -> String {
    if values.is_empty() {
        format!("this model takes no {name}; leave it out")
    } else {
        format!("{name} must be one of {}", values.join(", "))
    }
}

/// The model of `kind` to use: the one the user named, else the
/// conversation's default, else the catalog's first.
pub fn pick_model<'a>(
    models: &'a [Model],
    kind: Kind,
    named: Option<&str>,
    default: Option<&str>,
) -> Result<&'a Model, String> {
    let of_kind = |id: &str| models.iter().find(|m| m.id == id && m.kind == kind);
    let ids = || {
        models
            .iter()
            .filter(|m| m.kind == kind)
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>()
    };
    if let Some(named) = named.filter(|s| !s.is_empty()) {
        return of_kind(named).ok_or_else(|| {
            refuse(
                NO_MODEL,
                format!("{named} is not available; use one of {}", ids().join(", ")),
            )
        });
    }
    default
        .and_then(of_kind)
        .or_else(|| models.iter().find(|m| m.kind == kind && usable(m)))
        .ok_or_else(|| refuse(NO_MODEL, "no model of this kind is available"))
}

fn usable(model: &Model) -> bool {
    match model.kind {
        Kind::Image => model.image.as_ref().is_some_and(|c| c.generations),
        Kind::Video => model.video.is_some(),
        Kind::Speech => model.audio.is_some(),
        _ => false,
    }
}

fn prompt(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_PROMPT_CHARS {
        return Err(refuse(
            INVALID,
            format!("the prompt must have 1 to {MAX_PROMPT_CHARS} characters"),
        ));
    }
    Ok(text.to_owned())
}

/// A value the user named must be one the model offers; a default that is
/// not is dropped.
fn choose(
    name: &str,
    asked: Option<&String>,
    default: Option<&String>,
    offered: &[String],
) -> Result<Option<String>, String> {
    match asked {
        Some(value) if offered.contains(value) => Ok(Some(value.clone())),
        Some(_) => Err(refuse(INVALID, allowed(name, offered))),
        None => Ok(default.filter(|v| offered.contains(v)).cloned()),
    }
}

/// The image request for `args` on `model`: `made` images were already made
/// in this run. The input image, when any, is checked by the caller.
pub fn image_request(
    model: &Model,
    defaults: Option<&ImageDefaults>,
    args: &ImageArgs,
    made: usize,
) -> Result<(String, ImageRequest), String> {
    let caps = model
        .image
        .as_ref()
        .ok_or_else(|| refuse(NO_MODEL, "not an image model"))?;
    let prompt = prompt(&args.prompt)?;
    if args.image_ids.len() > 1 {
        return Err(refuse(INVALID, "change one image at a time"));
    }
    let edit = !args.image_ids.is_empty();
    if edit && !caps.edits {
        return Err(refuse(INVALID, "this model cannot change an image"));
    }
    if !edit && !caps.generations {
        return Err(refuse(INVALID, "this model only changes images"));
    }
    let same = defaults.filter(|d| d.model.as_deref().is_none_or(|m| m == model.id));
    let size = choose(
        "size",
        args.size.as_ref(),
        same.and_then(|d| d.size.as_ref()),
        &caps.sizes,
    )?;
    let quality = choose(
        "quality",
        args.quality.as_ref(),
        same.and_then(|d| d.quality.as_ref()),
        &caps.qualities,
    )?;
    let n = match args.n.or(same.and_then(|d| d.n)) {
        Some(n) if !caps.n && n > 1 => {
            if args.n.is_some() {
                return Err(refuse(INVALID, "this model makes one image per call"));
            }
            None
        }
        Some(n) if caps.n => Some(n),
        _ => None,
    };
    let count = n.unwrap_or(1) as usize;
    if count == 0 || made + count > MAX_IMAGES_PER_RUN {
        return Err(refuse(
            TURN_LIMIT,
            format!(
                "at most {MAX_IMAGES_PER_RUN} images per reply; {} left",
                MAX_IMAGES_PER_RUN.saturating_sub(made)
            ),
        ));
    }
    let request = ImageRequest {
        size,
        quality,
        n,
        input_attachment_id: args.image_ids.first().cloned(),
    };
    request
        .validate(caps)
        .map_err(|code| refuse(&code, "these parameters do not fit the model"))?;
    Ok((prompt, request))
}

/// The video request for `args` on `model`, frames from `plan`.
pub fn video_request(
    model: &Model,
    defaults: Option<&VideoDefaults>,
    args: &VideoArgs,
    plan: &materials::Plan,
) -> Result<(String, VideoRequest), String> {
    let caps = model
        .video
        .as_ref()
        .ok_or_else(|| refuse(NO_MODEL, "not a video model"))?;
    let prompt = prompt(&args.prompt)?;
    let mode = plan.mode();
    if !caps.modes.iter().any(|m| m == mode) {
        return Err(refuse(
            materials::UNSUPPORTED_MATERIAL,
            format!(
                "this model does not take these files (it supports {})",
                caps.modes.join(", ")
            ),
        ));
    }
    let same = defaults.filter(|d| d.model.as_deref().is_none_or(|m| m == model.id));
    let (min, max) = (
        caps.min_duration.unwrap_or(1),
        caps.max_duration.unwrap_or(0),
    );
    let in_range = |d: u64| d >= min && d <= max;
    let duration = match args.duration {
        Some(d) if in_range(d) => d,
        Some(_) => {
            return Err(refuse(
                INVALID,
                format!("duration must be {min} to {max} seconds"),
            ))
        }
        None => same
            .and_then(|d| d.duration)
            .filter(|d| in_range(*d))
            .unwrap_or_else(|| 5.clamp(min, max.max(min))),
    };
    let resolution = choose(
        "resolution",
        args.resolution.as_ref(),
        same.and_then(|d| d.resolution.as_ref()),
        &caps.resolutions,
    )?
    .or_else(|| caps.default_resolution.clone())
    .or_else(|| caps.resolutions.first().cloned())
    .ok_or_else(|| refuse(INVALID, "the model lists no resolution"))?;
    let aspect_ratio = choose(
        "aspect_ratio",
        args.aspect_ratio.as_ref(),
        same.and_then(|d| d.aspect_ratio.as_ref()),
        &caps.aspect_ratios,
    )?;
    let generate_audio = match args.generate_audio {
        Some(true) if !caps.supports_audio => {
            return Err(refuse(INVALID, "this model makes silent videos"))
        }
        Some(audio) => audio,
        None => same.and_then(|d| d.generate_audio).unwrap_or(true) && caps.supports_audio,
    };
    let request = VideoRequest {
        duration,
        resolution,
        aspect_ratio,
        generate_audio,
        first_frame_id: plan.first_frame.clone(),
        last_frame_id: plan.last_frame.clone(),
    };
    if plan.references.is_empty() {
        request
            .validate(caps)
            .map_err(|code| refuse(&code, "these parameters do not fit the model"))?;
    }
    Ok((prompt, request))
}

/// The speech request for `args` on `model`; ids are filled in by the host.
pub fn speech_choice(
    model: &Model,
    defaults: Option<&SpeechDefaults>,
    args: &SpeechArgs,
) -> Result<SpeechChoice, String> {
    let caps = model
        .audio
        .as_ref()
        .ok_or_else(|| refuse(NO_MODEL, "not a speech model"))?;
    let text = args.text.trim();
    if text.is_empty() || text.chars().count() > super::speech::MAX_INPUT_CHARS {
        return Err(refuse(
            INVALID,
            format!(
                "the text must have 1 to {} characters",
                super::speech::MAX_INPUT_CHARS
            ),
        ));
    }
    let same = defaults.filter(|d| d.model.as_deref().is_none_or(|m| m == model.id));
    let formats: Vec<String> = caps
        .response_formats
        .iter()
        .filter(|f| *f != "pcm")
        .cloned()
        .collect();
    let format = choose(
        "format",
        args.format.as_ref(),
        same.and_then(|d| d.format.as_ref()),
        &formats,
    )?
    .or_else(|| {
        formats
            .contains(&caps.default_format)
            .then(|| caps.default_format.clone())
    })
    .or_else(|| formats.first().cloned())
    .ok_or_else(|| refuse(INVALID, "the model lists no output format"))?;
    let voice = args
        .voice
        .clone()
        .filter(|v| !v.is_empty())
        .or_else(|| same.and_then(|d| d.voice.clone()))
        .or_else(|| caps.suggested_voices.first().cloned())
        .ok_or_else(|| refuse(INVALID, "name a voice"))?;
    let language = match args
        .language
        .as_ref()
        .or(same.and_then(|d| d.language.as_ref()))
    {
        Some(l) if ["zh", "en", "ja"].contains(&l.as_str()) && caps.language => Some(l.clone()),
        Some(_) if args.language.is_some() => {
            return Err(refuse(
                INVALID,
                "language must be zh, en or ja, or left out",
            ))
        }
        _ => None,
    };
    let style = args.style.clone().filter(|s| !s.trim().is_empty());
    if style.is_some() && !caps.instructions {
        return Err(refuse(INVALID, "this model takes no speaking style"));
    }
    Ok(SpeechChoice {
        text: text.to_owned(),
        voice,
        format,
        language,
        style,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechChoice {
    pub text: String,
    pub voice: String,
    pub format: String,
    pub language: Option<String>,
    pub style: Option<String>,
}

/// What a run tells the model about the generation tools: the default model
/// and what it offers, so the model passes only what the user asked for.
pub fn run_params(models: &[Model], defaults: &MediaDefaults) -> Value {
    let ids = |kind| {
        models
            .iter()
            .filter(|m| m.kind == kind && usable(m))
            .map(|m| m.id.clone())
            .collect::<Vec<_>>()
    };
    let mut params = serde_json::Map::new();
    let image = defaults.image.as_ref();
    if let Ok(model) = pick_model(
        models,
        Kind::Image,
        None,
        image.and_then(|d| d.model.as_deref()),
    ) {
        let caps = model.image.as_ref().expect("usable image model");
        params.insert(
            "image".into(),
            json!({"model":model.id,"models":ids(Kind::Image),"sizes":caps.sizes,
                "qualities":caps.qualities,"edits":caps.edits,"several":caps.n,
                "maxImages":MAX_IMAGES_PER_RUN}),
        );
    }
    let video = defaults.video.as_ref();
    if let Ok(model) = pick_model(
        models,
        Kind::Video,
        None,
        video.and_then(|d| d.model.as_deref()),
    ) {
        let caps = model.video.as_ref().expect("usable video model");
        params.insert(
            "video".into(),
            json!({"model":model.id,"models":ids(Kind::Video),"modes":caps.modes,
                "resolutions":caps.resolutions,"aspectRatios":caps.aspect_ratios,
                "minDuration":caps.min_duration,"maxDuration":caps.max_duration,
                "audio":caps.supports_audio}),
        );
    }
    let speech = defaults.speech.as_ref();
    if let Ok(model) = pick_model(
        models,
        Kind::Speech,
        None,
        speech.and_then(|d| d.model.as_deref()),
    ) {
        let caps = model.audio.as_ref().expect("usable speech model");
        params.insert(
            "speech".into(),
            json!({"model":model.id,"models":ids(Kind::Speech),
                "voices":caps.suggested_voices,"style":caps.instructions}),
        );
    }
    params.insert("hints".into(), json!(defaults.hints));
    Value::Object(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::assistant::catalog::{ImageCapabilities, VideoCapabilities};

    fn model(id: &str, kind: Kind) -> Model {
        let mut model: Model = serde_json::from_value(json!({
            "id": id, "name": id, "contextWindow": 1, "maxOutputTokens": 1,
            "supportsImages": false, "inputModalities": [], "supportedEndpoints": [],
            "inputPricePerMillion": null, "outputPricePerMillion": null,
            "pricing": {}, "cost": {}, "pricingKnown": false, "kind": "chat",
        }))
        .unwrap();
        model.kind = kind;
        model
    }
    fn image_model(id: &str) -> Model {
        let mut m = model(id, Kind::Image);
        m.image = Some(ImageCapabilities {
            generations: true,
            edits: true,
            n: true,
            sizes: vec!["1024x1024".into(), "1536x1024".into()],
            qualities: vec!["low".into(), "high".into()],
            per_image_price: Some("0.02".into()),
            token_pricing: None,
        });
        m
    }
    fn video_model(id: &str) -> Model {
        let mut m = model(id, Kind::Video);
        m.video = Some(VideoCapabilities {
            modes: vec!["t2v".into(), "i2v".into()],
            resolutions: vec!["480p".into(), "720p".into()],
            default_resolution: Some("720p".into()),
            min_duration: Some(4),
            max_duration: Some(15),
            supports_audio: true,
            aspect_ratios: vec!["16:9".into(), "9:16".into()],
            from_price_per_second: None,
        });
        m
    }
    fn image_args(prompt: &str) -> ImageArgs {
        serde_json::from_value(json!({ "prompt": prompt })).unwrap()
    }
    fn video_args(value: Value) -> VideoArgs {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_named_model_wins_then_the_default_then_the_first() {
        let models = vec![image_model("a"), image_model("b"), video_model("v")];
        assert_eq!(
            pick_model(&models, Kind::Image, Some("b"), Some("a"))
                .unwrap()
                .id,
            "b"
        );
        assert_eq!(
            pick_model(&models, Kind::Image, None, Some("b"))
                .unwrap()
                .id,
            "b"
        );
        assert_eq!(
            pick_model(&models, Kind::Image, None, Some("gone"))
                .unwrap()
                .id,
            "a"
        );
        assert_eq!(
            pick_model(&models, Kind::Image, None, None).unwrap().id,
            "a"
        );
        let named = pick_model(&models, Kind::Image, Some("v"), None).unwrap_err();
        assert!(
            named.starts_with(NO_MODEL) && named.contains("a, b"),
            "{named}"
        );
        assert!(pick_model(&models, Kind::Speech, None, None).is_err());
    }

    #[test]
    fn image_parameters_take_the_users_words_over_the_defaults() {
        let model = image_model("img");
        let defaults = ImageDefaults {
            model: Some("img".into()),
            size: Some("1536x1024".into()),
            quality: Some("high".into()),
            n: Some(2),
        };
        let (prompt, request) =
            image_request(&model, Some(&defaults), &image_args("  a cat  "), 0).unwrap();
        assert_eq!(prompt, "a cat");
        assert_eq!(request.size.as_deref(), Some("1536x1024"));
        assert_eq!(request.quality.as_deref(), Some("high"));
        assert_eq!(request.n, Some(2));
        let mut said = image_args("a cat");
        said.size = Some("1024x1024".into());
        said.n = Some(1);
        let (_, request) = image_request(&model, Some(&defaults), &said, 0).unwrap();
        assert_eq!(request.size.as_deref(), Some("1024x1024"));
        assert_eq!(request.n, Some(1));
    }

    #[test]
    fn defaults_of_another_model_are_not_applied() {
        let model = image_model("other");
        let defaults = ImageDefaults {
            model: Some("img".into()),
            size: Some("1536x1024".into()),
            ..Default::default()
        };
        let (_, request) = image_request(&model, Some(&defaults), &image_args("x"), 0).unwrap();
        assert_eq!(request.size, None);
    }

    #[test]
    fn a_named_value_the_model_does_not_offer_is_refused_with_the_choices() {
        let mut args = image_args("x");
        args.size = Some("640x480".into());
        let error = image_request(&image_model("img"), None, &args, 0).unwrap_err();
        assert!(
            error.starts_with(INVALID) && error.contains("1024x1024"),
            "{error}"
        );
        let defaults = ImageDefaults {
            size: Some("640x480".into()),
            ..Default::default()
        };
        let (_, request) =
            image_request(&image_model("img"), Some(&defaults), &image_args("x"), 0).unwrap();
        assert_eq!(request.size, None, "a stale default is dropped");
    }

    #[test]
    fn images_per_run_are_capped() {
        let model = image_model("img");
        assert!(image_request(&model, None, &image_args("x"), MAX_IMAGES_PER_RUN - 1).is_ok());
        let error = image_request(&model, None, &image_args("x"), MAX_IMAGES_PER_RUN).unwrap_err();
        assert!(error.starts_with(TURN_LIMIT), "{error}");
        let mut many = image_args("x");
        many.n = Some(3);
        assert!(image_request(&model, None, &many, 2)
            .unwrap_err()
            .starts_with(TURN_LIMIT));
    }

    #[test]
    fn an_edit_takes_one_image_on_a_model_that_edits() {
        let mut args = image_args("make it blue");
        args.image_ids = vec!["a".into()];
        let (_, request) = image_request(&image_model("img"), None, &args, 0).unwrap();
        assert_eq!(request.input_attachment_id.as_deref(), Some("a"));
        args.image_ids.push("b".into());
        assert!(image_request(&image_model("img"), None, &args, 0).is_err());
        let mut no_edits = image_model("img");
        no_edits.image.as_mut().unwrap().edits = false;
        args.image_ids.pop();
        assert!(image_request(&no_edits, None, &args, 0).is_err());
    }

    #[test]
    fn an_empty_or_huge_prompt_is_refused() {
        assert!(image_request(&image_model("i"), None, &image_args("  "), 0).is_err());
        let long = "x".repeat(MAX_PROMPT_CHARS + 1);
        assert!(image_request(&image_model("i"), None, &image_args(&long), 0).is_err());
    }

    #[test]
    fn video_defaults_fill_what_the_user_did_not_say_and_sound_is_on() {
        let model = video_model("vid");
        let defaults = VideoDefaults {
            model: Some("vid".into()),
            duration: Some(8),
            resolution: Some("480p".into()),
            aspect_ratio: Some("9:16".into()),
            generate_audio: None,
        };
        let plan = materials::Plan::default();
        let (_, request) = video_request(
            &model,
            Some(&defaults),
            &video_args(json!({"prompt":"waves"})),
            &plan,
        )
        .unwrap();
        assert_eq!((request.duration, request.resolution.as_str()), (8, "480p"));
        assert_eq!(request.aspect_ratio.as_deref(), Some("9:16"));
        assert!(request.generate_audio);
        let (_, said) = video_request(
            &model,
            Some(&defaults),
            &video_args(json!({"prompt":"waves","duration":4,"generate_audio":false})),
            &plan,
        )
        .unwrap();
        assert_eq!(said.duration, 4);
        assert!(!said.generate_audio);
        let (_, bare) =
            video_request(&model, None, &video_args(json!({"prompt":"x"})), &plan).unwrap();
        assert_eq!((bare.duration, bare.resolution.as_str()), (5, "720p"));
    }

    #[test]
    fn a_video_out_of_range_or_with_files_the_model_cannot_take_is_refused() {
        let model = video_model("vid");
        let plan = materials::Plan::default();
        let error = video_request(
            &model,
            None,
            &video_args(json!({"prompt":"x","duration":60})),
            &plan,
        )
        .unwrap_err();
        assert!(error.contains("4 to 15"), "{error}");
        let with_video = materials::plan(
            &[materials::Material {
                id: "m".into(),
                kind: materials::Kind::Video,
                duration_ms: None,
            }],
            materials::Use::Auto,
        )
        .unwrap();
        let error = video_request(
            &model,
            None,
            &video_args(json!({"prompt":"x"})),
            &with_video,
        )
        .unwrap_err();
        assert!(
            error.starts_with(materials::UNSUPPORTED_MATERIAL),
            "{error}"
        );
    }

    #[test]
    fn a_first_frame_plan_becomes_the_request_frames() {
        let (start, end) = (
            crate::services::assistant::types::id(),
            crate::services::assistant::types::id(),
        );
        let plan = materials::Plan {
            first_frame: Some(start.clone()),
            last_frame: Some(end.clone()),
            references: vec![],
        };
        let (_, request) = video_request(
            &video_model("vid"),
            None,
            &video_args(json!({"prompt":"x"})),
            &plan,
        )
        .unwrap();
        assert_eq!(request.first_frame_id, Some(start));
        assert_eq!(request.last_frame_id, Some(end));
    }

    #[test]
    fn tool_arguments_are_strict() {
        assert!(serde_json::from_value::<ImageArgs>(json!({"prompt":"x","secret":1})).is_err());
        let args = video_args(json!({"prompt":"x","material_ids":["a"],"material_use":"frames"}));
        assert_eq!(args.material_use, materials::Use::Frames);
    }

    #[test]
    fn defaults_are_bounded() {
        let mut defaults = MediaDefaults {
            hints: vec!["image".into()],
            ..Default::default()
        };
        assert!(defaults.validate().is_ok());
        defaults.hints.push("music".into());
        assert!(defaults.validate().is_err());
        let parsed: Result<MediaDefaults, _> =
            serde_json::from_value(json!({"image":{"model":"x","unknown":1}}));
        assert!(parsed.is_err());
    }

    #[test]
    fn run_params_name_the_default_model_and_its_choices() {
        let models = vec![image_model("a"), image_model("b"), video_model("v")];
        let defaults = MediaDefaults {
            image: Some(ImageDefaults {
                model: Some("b".into()),
                ..Default::default()
            }),
            hints: vec!["video".into()],
            ..Default::default()
        };
        let params = run_params(&models, &defaults);
        assert_eq!(params["image"]["model"], "b");
        assert_eq!(params["image"]["models"], json!(["a", "b"]));
        assert_eq!(params["video"]["model"], "v");
        assert!(params.get("speech").is_none());
        assert_eq!(params["hints"], json!(["video"]));
    }
}
