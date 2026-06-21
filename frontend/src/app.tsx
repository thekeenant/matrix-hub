import { useState, useEffect } from 'preact/hooks';
import { DeviceConfig, UpdateConfigRequest } from './generated/proto/config';
import './app.css';

type ToastType = 'success' | 'error' | 'info' | null;

export function App() {
  const [config, setConfig] = useState<DeviceConfig>({
    wifi: { ssid: '', pass: '' },
    brightness: 128,
  });
  const [loading, setLoading] = useState(true);
  const [savingWifi, setSavingWifi] = useState(false);
  const [savingBrightness, setSavingBrightness] = useState(false);
  const [toast, setToast] = useState<{ message: string; type: ToastType }>({ message: '', type: null });

  useEffect(() => {
    fetchConfig();
  }, []);

  const showToast = (message: string, type: ToastType) => {
    setToast({ message, type });
    setTimeout(() => setToast({ message: '', type: null }), 5000);
  };

  const fetchConfig = async () => {
    try {
      const response = await fetch('/api/config');
      if (!response.ok) throw new Error('Failed to fetch config');
      const buffer = await response.arrayBuffer();
      const decoded = DeviceConfig.decode(new Uint8Array(buffer));
      
      if (!decoded.wifi) decoded.wifi = { ssid: '', pass: '' };
      
      setConfig(decoded);
      setLoading(false);
    } catch (err) {
      console.error(err);
      setLoading(false);
      showToast('Could not load config (are you running on the ESP32?)', 'info');
    }
  };

  const savePartialConfig = async (mask: string[], isWifi: boolean) => {
    if (isWifi) setSavingWifi(true);
    else setSavingBrightness(true);
    setToast({ message: '', type: null });

    try {
      const req = UpdateConfigRequest.create({
        config: config,
        updateMask: mask,
      });
      const binaryData = UpdateConfigRequest.encode(req).finish();
      const response = await fetch('/api/config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/octet-stream' },
        body: binaryData,
      });

      if (!response.ok) throw new Error('Failed to save config');
      showToast(`${isWifi ? 'WiFi' : 'Brightness'} settings saved!`, 'success');
    } catch (err) {
      console.error(err);
      showToast('Error saving settings. Please try again.', 'error');
    } finally {
      if (isWifi) setSavingWifi(false);
      else setSavingBrightness(false);
    }
  };

  if (loading) {
    return <div class="glass-panel"><p style={{textAlign: 'center'}}>Loading Matrix Hub...</p></div>;
  }

  return (
    <div class="glass-panel">
      <h1>Matrix Hub</h1>
      
      {toast.type && (
        <div class={`toast ${toast.type}`}>
          {toast.message}
        </div>
      )}

      <form onSubmit={(e) => { e.preventDefault(); savePartialConfig(['wifi'], true); }}>
        <h2 style={{ fontSize: '1.2rem', marginTop: 0, marginBottom: '16px', color: '#38bdf8' }}>WiFi Network</h2>
        <div class="form-group">
          <label htmlFor="ssid">WiFi SSID</label>
          <input
            id="ssid"
            type="text"
            value={config.wifi?.ssid || ''}
            onInput={(e) => setConfig({ ...config, wifi: { ...config.wifi!, ssid: (e.target as HTMLInputElement).value } })}
            placeholder="Network Name"
            required
          />
        </div>

        <div class="form-group">
          <label htmlFor="pass">WiFi Password</label>
          <input
            id="pass"
            type="password"
            value={config.wifi?.pass || ''}
            onInput={(e) => setConfig({ ...config, wifi: { ...config.wifi!, pass: (e.target as HTMLInputElement).value } })}
            placeholder="Password (leave blank if open)"
          />
        </div>

        <button type="submit" disabled={savingWifi}>
          {savingWifi ? 'Saving...' : 'Update WiFi'}
        </button>
      </form>

      <div style={{ height: '1px', background: 'rgba(255,255,255,0.1)', margin: '30px 0' }}></div>

      <form onSubmit={(e) => { e.preventDefault(); savePartialConfig(['brightness'], false); }}>
        <h2 style={{ fontSize: '1.2rem', marginTop: 0, marginBottom: '16px', color: '#38bdf8' }}>Display</h2>
        <div class="form-group">
          <label htmlFor="brightness">Global Brightness (0-255)</label>
          <input
            id="brightness"
            type="number"
            min="0"
            max="255"
            value={config.brightness}
            onInput={(e) => setConfig({ ...config, brightness: parseInt((e.target as HTMLInputElement).value) || 0 })}
            required
          />
        </div>

        <button type="submit" disabled={savingBrightness}>
          {savingBrightness ? 'Saving...' : 'Update Display'}
        </button>
      </form>
    </div>
  );
}
