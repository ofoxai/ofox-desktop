//! Shared admission and receipts for independent audio tools.
use super::*;
use crate::services::assistant::{
    audio_input::{Input, Preview},
    catalog::{Kind, Model},
    host::{scrub_secrets, AssistantHost},
    speech::{self, SpeechRequest},
    store::SessionRecord,
};
use std::time::Instant;
#[derive(Clone)]
pub(super) enum Payload {
    Transcription(TranscribeRequest),
    Speech(SpeechRequest),
}
#[derive(Clone)]
pub(super) struct Job {
    pub session_id: String,
    pub expected_owner: String,
    pub expected_generation: u64,
    pub run_id: String,
    pub operation_id: String,
    pub model_id: String,
    pub payload: Payload,
}
macro_rules! job_from {
    ($request:ty,$variant:ident) => {
        impl From<$request> for Job {
            fn from(r: $request) -> Self {
                Self {
                    session_id: r.session_id.clone(),
                    expected_owner: r.expected_owner.clone(),
                    expected_generation: r.expected_generation,
                    run_id: r.run_id.clone(),
                    operation_id: r.operation_id.clone(),
                    model_id: r.model_id.clone(),
                    payload: Payload::$variant(r),
                }
            }
        }
    };
}
job_from!(TranscribeRequest, Transcription);
job_from!(SpeechRequest, Speech);
pub(super) struct Completion {
    pub text: Option<String>,
    pub usage: Value,
    pub request_id: Option<String>,
    pub request_id_source: Option<String>,
    pub output: Option<(Vec<u8>, speech::OutputInfo)>,
}
impl Job {
    pub fn authority(&self) -> DeleteRequest {
        DeleteRequest {
            session_id: self.session_id.clone(),
            expected_owner: self.expected_owner.clone(),
            expected_generation: self.expected_generation,
        }
    }
    pub fn kind(&self) -> Kind {
        match self.payload {
            Payload::Transcription(_) => Kind::Transcription,
            Payload::Speech(_) => Kind::Speech,
        }
    }
    pub fn preview(&self, host: &AssistantHost) -> Result<Option<Preview>, String> {
        match &self.payload {
            Payload::Speech(_) => Ok(None),
            Payload::Transcription(r) => host
                .audio_inputs
                .lock()
                .map_err(|_| "assistant_state_unavailable")?
                .preview(&self.authority(), &r.input_id, Instant::now())
                .map(Some),
        }
    }
    pub fn take(&self, host: &AssistantHost) -> Result<Option<Input>, String> {
        match &self.payload {
            Payload::Speech(_) => Ok(None),
            Payload::Transcription(r) => host
                .audio_inputs
                .lock()
                .map_err(|_| "assistant_state_unavailable")?
                .take(&self.authority(), &r.input_id, Instant::now())
                .map(Some),
        }
    }
    pub fn keep_original(&self) -> bool {
        matches!(&self.payload, Payload::Transcription(r) if r.keep_original)
    }
    pub fn speech_choice(&self) -> Option<(String, String)> {
        match &self.payload {
            Payload::Speech(r) => Some((r.voice.clone(), r.response_format.clone())),
            _ => None,
        }
    }
    pub fn check(&self, record: &SessionRecord, model: &Model) -> Result<(), String> {
        if self.keep_original() {
            crate::services::assistant::attachments::original_audio_room(record)?;
        }
        if let Payload::Speech(r) = &self.payload {
            r.body(model)?;
            if let Some(mid) = &r.source_message_id {
                if !record.messages.iter().any(|m| {
                    m.id == *mid
                        && m.role == "assistant"
                        && m.status == "completed"
                        && !m.superseded
                        && m.media.is_none()
                        && m.content == r.input
                }) {
                    return Err("assistant_message_not_found".into());
                }
            }
            crate::services::assistant::attachments::audio_room(record)?;
        }
        Ok(())
    }
    pub fn scrub(&mut self, secrets: &[String]) {
        if let Payload::Speech(r) = &mut self.payload {
            r.input = scrub_secrets(&r.input, secrets);
            r.voice = scrub_secrets(&r.voice, secrets);
            if let Some(s) = &mut r.instructions {
                *s = scrub_secrets(s, secrets);
            }
        }
    }
    pub fn media(
        &self,
        model: &Model,
        preview: Option<Preview>,
        apex: &str,
    ) -> Result<MediaRecord, String> {
        let (kind, params, text, estimate, input) = match &self.payload {
            Payload::Transcription(r) => {
                let p = preview.ok_or(INPUT_UNAVAILABLE)?;
                (
                    "transcription",
                    serde_json::json!({"language":r.language,"inputName":p.name,"keepOriginal":r.keep_original}),
                    None,
                    None,
                    Some(p.info),
                )
            }
            Payload::Speech(r) => {
                r.body(model)?;
                (
                    "speech",
                    serde_json::json!({"voice":r.voice,"responseFormat":r.response_format,"language":r.language,"instructions":r.instructions,"sourceMessageId":r.source_message_id}),
                    Some(r.input.clone()),
                    speech::estimate(model, &r.input),
                    None,
                )
            }
        };
        Ok(MediaRecord {
            kind: kind.into(),
            model: model.id.clone(),
            apex: Some(apex.into()),
            params,
            status: "not_sent".into(),
            cost: Some("0".into()),
            audio: Some(Trace {
                pricing: model.audio.as_ref().ok_or(INVALID)?.pricing.clone(),
                input,
                text,
                usage: Value::Null,
                request_id: None,
                request_id_source: None,
                error_code: None,
                original_error: None,
                estimate,
                output: None,
            }),
            ..Default::default()
        })
    }
    pub async fn perform(
        &self,
        client: &reqwest::Client,
        gateway: &str,
        key: &str,
        model: &Model,
        input: Option<&Input>,
    ) -> Result<Completion, http::Failure> {
        match &self.payload {
            Payload::Transcription(r) => {
                let input = input.ok_or(http::Failure {
                    code: INPUT_UNAVAILABLE,
                    unbilled: true,
                    request_id: None,
                    request_id_source: None,
                })?;
                let response =
                    http::transcribe(client, gateway, key, model, input, r.language.as_deref())
                        .await?;
                Ok(Completion {
                    text: Some(response.text),
                    usage: response.usage,
                    request_id: response.request_id,
                    request_id_source: response.request_id_source,
                    output: None,
                })
            }
            Payload::Speech(r) => {
                let response = speech::http::synthesize(client, gateway, key, model, r)
                    .await
                    .map_err(|e| http::Failure {
                        code: e.code,
                        unbilled: e.unbilled,
                        request_id: e.request_id,
                        request_id_source: e.request_id_source,
                    })?;
                Ok(Completion {
                    text: Some(r.input.clone()),
                    usage: Value::Null,
                    request_id: response.request_id,
                    request_id_source: response.request_id_source,
                    output: Some((response.bytes, response.info)),
                })
            }
        }
    }
}
