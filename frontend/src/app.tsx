import { useEffect, useState } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { DeviceConfig } from './generated/proto/config';
import { fetchConfig, saveConfig, type ConfigField } from './api';

type ToastType = 'success' | 'error' | 'info';
type Toast = { message: string; type: ToastType } | null;
type Notify = (message: string, type: ToastType) => void;

const DEFAULT_CONFIG: DeviceConfig = {
  wifi: { ssid: '', pass: '' },
  brightness: 128,
  minMinutes: 1,
};

export function App() {
  const [config, setConfig] = useState<DeviceConfig>(DEFAULT_CONFIG);
  const [loading, setLoading] = useState(true);
  const [toast, setToast] = useState<Toast>(null);

  const notify: Notify = (message, type) => {
    setToast({ message, type });
    setTimeout(() => setToast(null), 5000);
  };

  useEffect(() => {
    fetchConfig()
      .then(setConfig)
      .catch((err) => {
        console.error(err);
        notify('Could not load config (are you running on the device?)', 'info');
      })
      .finally(() => setLoading(false));
  }, []);

  if (loading) {
    return (
      <div class="glass-panel">
        <p class="loading">Loading Matrix Hub...</p>
      </div>
    );
  }

  const update = (patch: Partial<DeviceConfig>) => setConfig({ ...config, ...patch });

  return (
    <div class="glass-panel">
      <h1>Matrix Hub</h1>

      {toast && <div class={`toast ${toast.type}`}>{toast.message}</div>}

      <SettingsForm
        title="WiFi Network"
        submitLabel="Update WiFi"
        config={config}
        fields={['wifi']}
        notify={notify}
      >
        <Field label="WiFi SSID" id="ssid">
          <input
            id="ssid"
            type="text"
            value={config.wifi?.ssid ?? ''}
            onInput={(e) => update({ wifi: { pass: '', ...config.wifi, ssid: e.currentTarget.value } })}
            placeholder="Network Name"
            maxLength={32}
            required
          />
        </Field>
        <Field label="WiFi Password" id="pass">
          <input
            id="pass"
            type="password"
            value={config.wifi?.pass ?? ''}
            onInput={(e) => update({ wifi: { ssid: '', ...config.wifi, pass: e.currentTarget.value } })}
            placeholder="Password (leave blank if open)"
            maxLength={64}
          />
        </Field>
      </SettingsForm>

      <hr class="divider" />

      <SettingsForm
        title="Display"
        submitLabel="Update Display"
        config={config}
        fields={['brightness', 'min_minutes']}
        notify={notify}
      >
        <Field label="Global Brightness (1-255)" id="brightness">
          <NumberInput
            id="brightness"
            min={1}
            max={255}
            value={config.brightness}
            onChange={(brightness) => update({ brightness })}
          />
        </Field>
        <Field label="Minimum Minutes to Show (0-60)" id="minMinutes">
          <NumberInput
            id="minMinutes"
            min={0}
            max={60}
            value={config.minMinutes ?? 1}
            onChange={(minMinutes) => update({ minMinutes })}
          />
        </Field>
      </SettingsForm>
    </div>
  );
}

/** A form section that saves only its own `fields` of the config. */
function SettingsForm(props: {
  title: string;
  submitLabel: string;
  config: DeviceConfig;
  fields: ConfigField[];
  notify: Notify;
  children: ComponentChildren;
}) {
  const [saving, setSaving] = useState(false);

  const onSubmit = async (e: Event) => {
    e.preventDefault();
    setSaving(true);
    try {
      await saveConfig(props.config, props.fields);
      props.notify(`${props.title} settings saved!`, 'success');
    } catch (err) {
      console.error(err);
      props.notify(`Error saving ${props.title} settings: ${(err as Error).message}`, 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <form onSubmit={onSubmit}>
      <h2 class="section-title">{props.title}</h2>
      {props.children}
      <button type="submit" disabled={saving}>
        {saving ? 'Saving...' : props.submitLabel}
      </button>
    </form>
  );
}

function Field(props: { label: string; id: string; children: ComponentChildren }) {
  return (
    <div class="form-group">
      <label htmlFor={props.id}>{props.label}</label>
      {props.children}
    </div>
  );
}

function NumberInput(props: {
  id: string;
  min: number;
  max: number;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <input
      id={props.id}
      type="number"
      min={props.min}
      max={props.max}
      value={props.value}
      onInput={(e) => {
        const n = parseInt(e.currentTarget.value, 10);
        props.onChange(Number.isNaN(n) ? props.min : Math.min(props.max, Math.max(props.min, n)));
      }}
      required
    />
  );
}
