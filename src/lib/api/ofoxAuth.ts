import { invoke } from "@tauri-apps/api/core";

export interface OfoxUserInfo {
  email: string | null;
  name: string | null;
  org_id: string | null;
  avatar_url: string | null;
}

export interface OfoxDeviceCodeResponse {
  device_code: string;
  user_code: string;
  verification_uri: string;
  verification_uri_complete: string | null;
  expires_in: number;
  interval: number;
}

export async function ofoxStartLogin(): Promise<OfoxDeviceCodeResponse> {
  return invoke<OfoxDeviceCodeResponse>("ofox_start_login");
}

export async function ofoxPollForToken(
  deviceCode: string,
): Promise<OfoxUserInfo | null> {
  return invoke<OfoxUserInfo | null>("ofox_poll_for_token", { deviceCode });
}

export async function ofoxGetUserInfo(): Promise<OfoxUserInfo | null> {
  return invoke<OfoxUserInfo | null>("ofox_get_user_info");
}

export async function ofoxIsAuthenticated(): Promise<boolean> {
  return invoke<boolean>("ofox_is_authenticated");
}

export async function ofoxLogout(): Promise<void> {
  return invoke("ofox_logout");
}
