import { DeviceConfig, UpdateConfigRequest } from './generated/proto/config';

/** Fields of DeviceConfig that can be updated independently (AIP-134 mask). */
export type ConfigField = 'wifi' | 'brightness' | 'min_minutes';

export async function fetchConfig(): Promise<DeviceConfig> {
  const response = await fetch('/api/config');
  if (!response.ok) throw new Error(`Failed to fetch config (HTTP ${response.status})`);
  const config = DeviceConfig.decode(new Uint8Array(await response.arrayBuffer()));
  config.wifi ??= { ssid: '', pass: '' };
  return config;
}

/** Saves only the fields listed in `mask`. Throws with the device's message on failure. */
export async function saveConfig(config: DeviceConfig, mask: ConfigField[]): Promise<void> {
  const body = UpdateConfigRequest.encode(
    UpdateConfigRequest.create({ config, updateMask: mask }),
  ).finish();
  const response = await fetch('/api/config', {
    method: 'POST',
    headers: { 'Content-Type': 'application/octet-stream' },
    body,
  });
  if (!response.ok) {
    throw new Error((await response.text()) || `HTTP ${response.status}`);
  }
}
