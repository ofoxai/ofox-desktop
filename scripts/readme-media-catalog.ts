/** Synthetic catalog entries for existing generation controls, never live models. */
import type { AssistantModel } from "@/lib/api/assistant";

const common = {
  contextWindow: 0,
  maxOutputTokens: 0,
  inputPricePerMillion: null,
  outputPricePerMillion: null,
  pricing: {},
  cost: { input: null, output: null, cacheRead: null, cacheWrite: null },
  pricingKnown: false,
  protocol: null,
  tools: false,
  reasoning: false,
  thinkingLevels: [],
};

export const readmeImageModel: AssistantModel = {
  ...common,
  id: "demo/image",
  name: "Demo Image",
  shortName: "Demo Image",
  vendor: "demo",
  kind: "image",
  supportsImages: true,
  inputModalities: ["text", "image"],
  supportedEndpoints: ["images/generations", "images/edits"],
  image: {
    generations: true,
    edits: true,
    n: true,
    sizes: ["1024x1024", "1536x1024", "1024x1536"],
    qualities: ["auto", "low", "medium", "high"],
    perImagePrice: null,
    tokenPricing: null,
  },
};

export const readmeVideoModel: AssistantModel = {
  ...common,
  id: "demo/video",
  name: "Demo Video",
  shortName: "Demo Video",
  vendor: "demo",
  kind: "video",
  supportsImages: true,
  inputModalities: ["text", "image"],
  supportedEndpoints: ["videos"],
  video: {
    modes: ["t2v", "i2v"],
    resolutions: ["480p", "720p", "1080p"],
    defaultResolution: "720p",
    minDuration: 4,
    maxDuration: 8,
    supportsAudio: true,
    aspectRatios: ["16:9", "9:16", "1:1"],
    fromPricePerSecond: null,
  },
};

export const readmeSpeechModel: AssistantModel = {
  ...common,
  id: "demo/speech",
  name: "Demo Speech",
  shortName: "Demo Speech",
  vendor: "demo",
  kind: "speech",
  supportsImages: false,
  inputModalities: ["text"],
  supportedEndpoints: ["audio/speech"],
  audio: {
    family: "gemini-tts",
    responseFormats: ["wav", "pcm"],
    defaultFormat: "wav",
    voiceKind: "name",
    suggestedVoices: ["Calm", "Warm", "Bright"],
    instructions: true,
    prompt: false,
    language: false,
    wordTimestamps: false,
    pricing: {
      unit: "tokens",
      inputAudioPerSecond: null,
      inputTextPerCharacter: null,
      inputTextPerMillion: null,
      inputAudioPerMillion: null,
      outputTextPerMillion: null,
      outputAudioPerMillion: null,
    },
  },
};
