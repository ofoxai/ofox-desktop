import { Type } from "typebox";

// Generation tools: the host makes the image, video or speech with the
// conversation's defaults and keeps the files; the model gets ids and a
// status, never bytes. `params.media` names what each tool may use.

const MAX_PROMPT = 4000;

const optionalChoice = (values, description) =>
  Type.Optional(
    values?.length
      ? Type.String({ enum: values, description })
      : Type.String({ maxLength: 64, description }),
  );

const modelChoice = (kind) =>
  Type.Optional(
    Type.String({
      enum: kind.models,
      description: `Only when the user names a model; otherwise ${kind.model} is used.`,
    }),
  );

const fileList = (files) =>
  (files ?? [])
    .map((f) => `${f.name ?? f.id} (attachment id ${f.id})`)
    .join(", ");

function describe(name, result, presentation) {
  const cost = result?.estimatedCostUsd
    ? ` Estimated cost: $${result.estimatedCostUsd}.`
    : "";
  if (name === "generate_video")
    return `Video queued with ${result?.model}: it is being generated and will appear under your reply in a few minutes, where the user can watch it. Do not call generate_video again for this request.${cost}`;
  if (name === "generate_speech")
    return `Speech generated with ${result?.model}: ${fileList(result?.files)}. The user sees a player under your reply; do not repeat the audio as a link.${cost}`;
  const use = presentation
    ? `For the frozen presentation, embed with image(slide, ${JSON.stringify(presentation.presentationAssetId)}, x, y, w, h). This is the PPT asset ID; the attachment ID is for chat/image editing, not this lookup.`
    : "Use the attachment ids to change an image or to start a video from it.";
  return `Generated with ${result?.model}: ${fileList(result?.files)}. The images are shown under your reply; do not repeat them as links. ${use}${cost}`;
}

function guidelines(media) {
  const lines = [
    "Generation tools make what the user asks for; the user's own words decide what to make.",
    "Pass the user's description faithfully in the prompt, in the language that suits the model; do not add subjects, styles or details the user did not ask for, and do not drop any they did.",
    "Pass only the parameters the user stated in this conversation (model, size, quality, count, duration, resolution, aspect ratio, sound, voice); leave the rest out so the defaults set under the composer apply.",
    'Images, audio and videos the user attached are named as [User attachment: "name"; id=...]; files you generated are named by attachment id in the tool result. Use those ids.',
  ];
  if (media.image)
    lines.push(
      `generate_image draws with ${media.image.model} by default; pass image_ids with one image to change that image instead. At most ${media.image.maxImages} images per reply.`,
      "A flowchart, sequence diagram, org chart, mind map, timeline or other diagram of steps or structure is a mermaid block in your reply, not generate_image: its text stays exact and it costs nothing. Use generate_image for one only when the user asks for a picture or illustration of it, or turned on image generation.",
    );
  if (media.video)
    lines.push(
      `generate_video uses ${media.video.model} by default. Files in material_ids: one image is the first frame; several images, or any audio or video, are references for subject and style. Use material_use "frames" when the user says an image should open or end the video (first image opens, second ends), and "reference" when they want an image as a guide rather than a frame. References take at most 9 images, 3 audio clips of up to 15 seconds and 1 video. Each video costs money and the user is asked to confirm it.`,
    );
  if (media.speech)
    lines.push(
      `generate_speech reads text aloud with ${media.speech.model}. For a video with dialogue or a given voice, make the speech first and pass its attachment id to generate_video in material_ids.`,
    );
  for (const hint of media.hints ?? [])
    lines.push(
      `The user turned on ${hint} generation in the composer: when they describe what they want, call generate_${hint}.`,
    );
  return lines;
}

export function mediaTools(runtime, context) {
  const media = context.params.media ?? {};
  const planned = context.params.documents?.presentationPlanning === "v1";
  const host = (name) => async (args, signal, toolCallId) => {
    if (
      name === "generate_image" &&
      planned &&
      ["presentationAssetId", "planId", "revision"].some(
        (key) => args[key] !== undefined,
      ) &&
      (!/^[A-Za-z0-9_-]{1,80}$/.test(args.presentationAssetId ?? "") ||
        !/^[A-Za-z0-9_-]{1,80}$/.test(args.planId ?? "") ||
        !Number.isSafeInteger(args.revision) ||
        args.revision < 1)
    )
      throw new Error(
        "assistant_presentation_asset_invalid: PPT image generation requires a frozen planId, revision and presentationAssetId; Host budget approval is separate",
      );
    const presentation =
      name === "generate_image" &&
      planned &&
      ["presentationAssetId", "planId", "revision"].some(
        (key) => args[key] !== undefined,
      )
        ? {
            presentationAssetId: args.presentationAssetId,
            planId: args.planId,
            revision: args.revision,
          }
        : undefined;
    let result;
    try {
      result = await runtime.hostTool(context, name, args, signal, toolCallId);
    } catch (error) {
      // A deck does not end over one picture: the Host has already counted
      // the image against the budget, so only this image is lost.
      if (
        !presentation ||
        !/\bassistant_media_outcome_unknown\b/.test(String(error?.message))
      )
        throw error;
      return {
        content: [
          {
            type: "text",
            text: `No answer arrived for image ${presentation.presentationAssetId}; it may have been charged and counts against the image budget. Do not request it again: build the page without it, using the plan's fallback.`,
          },
        ],
        details: { code: "assistant_media_unanswered" },
        isError: true,
      };
    }
    if (
      presentation &&
      (result?.status !== "completed" ||
        result.presentationAssetId !== presentation.presentationAssetId ||
        result.planId !== presentation.planId ||
        result.revision !== presentation.revision)
    )
      throw new Error(
        "assistant_presentation_asset_invalid: generated image result does not match the frozen presentation request",
      );
    return {
      content: [{ type: "text", text: describe(name, result, presentation) }],
      details: result,
    };
  };
  const tools = [];
  if (media.image) {
    const image = media.image;
    tools.push(
      runtime.wrapTool(
        context,
        "generate_image",
        "Create an image from a description, or change an attached or generated image. The image appears under your reply.",
        Type.Object({
          prompt: Type.String({ minLength: 1, maxLength: MAX_PROMPT }),
          model: modelChoice(image),
          size: optionalChoice(
            image.sizes,
            "Only when the user states a size, or for a planned presentation image the size the frozen plan lists for that asset in imageSizes.",
          ),
          quality: optionalChoice(
            image.qualities,
            "Only when the user states a quality.",
          ),
          n: Type.Optional(
            Type.Integer({
              minimum: 1,
              maximum: image.several ? image.maxImages : 1,
              description: "How many images, when the user asks for several.",
            }),
          ),
          image_ids: Type.Optional(
            Type.Array(Type.String({ minLength: 1, maxLength: 200 }), {
              maxItems: 1,
              description: "The attachment id of an image to change.",
            }),
          ),
          ...(planned
            ? {
                presentationAssetId: Type.Optional(
                  Type.String({
                    pattern: "^[A-Za-z0-9_-]{1,80}$",
                    description:
                      "The generated-image visual.assetId of the frozen page plan; does not grant a budget.",
                  }),
                ),
                planId: Type.Optional(
                  Type.String({ pattern: "^[A-Za-z0-9_-]{1,80}$" }),
                ),
                revision: Type.Optional(Type.Integer({ minimum: 1 })),
              }
            : {}),
        }),
        host("generate_image"),
      ),
    );
  }
  if (media.video) {
    const video = media.video;
    tools.push(
      runtime.wrapTool(
        context,
        "generate_video",
        "Generate a short video from a description, optionally starting from images, audio or a video the user gave or you generated. It takes minutes and appears under your reply when done.",
        Type.Object({
          prompt: Type.String({ minLength: 1, maxLength: MAX_PROMPT }),
          model: modelChoice(video),
          duration: Type.Optional(
            Type.Integer({
              minimum: video.minDuration ?? 1,
              maximum: video.maxDuration ?? 60,
              description: "Seconds, only when the user states a length.",
            }),
          ),
          resolution: optionalChoice(
            video.resolutions,
            "Only when the user states a resolution.",
          ),
          aspect_ratio: optionalChoice(
            video.aspectRatios,
            "Only when the user states an aspect ratio or orientation.",
          ),
          ...(video.audio
            ? {
                generate_audio: Type.Optional(
                  Type.Boolean({
                    description:
                      "false only when the user wants a silent video.",
                  }),
                ),
              }
            : {}),
          material_ids: Type.Optional(
            Type.Array(Type.String({ minLength: 1, maxLength: 200 }), {
              maxItems: 13,
              description: "Attachment ids of images, audio or video to use.",
            }),
          ),
          material_use: Type.Optional(
            Type.String({ enum: ["auto", "frames", "reference"] }),
          ),
        }),
        host("generate_video"),
      ),
    );
  }
  if (media.speech) {
    const speech = media.speech;
    tools.push(
      runtime.wrapTool(
        context,
        "generate_speech",
        "Read text aloud as an audio file, for narration or a voice-over. The audio appears under your reply.",
        Type.Object({
          text: Type.String({ minLength: 1, maxLength: MAX_PROMPT }),
          model: modelChoice(speech),
          voice: Type.Optional(
            Type.String({
              maxLength: 64,
              description: speech.voices?.length
                ? `Only when the user asks for a voice. Known voices: ${speech.voices.join(", ")}.`
                : "Only when the user asks for a voice.",
            }),
          ),
          language: Type.Optional(Type.String({ enum: ["zh", "en", "ja"] })),
          ...(speech.style
            ? {
                style: Type.Optional(
                  Type.String({
                    maxLength: 1000,
                    description:
                      "How to speak (tone, pace), when the user describes it.",
                  }),
                ),
              }
            : {}),
        }),
        host("generate_speech"),
      ),
    );
  }
  if (tools.length)
    tools[0] = { ...tools[0], promptGuidelines: guidelines(media) };
  return tools;
}
