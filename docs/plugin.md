# DevicePlugin Trait Reference

Complete reference for the `DevicePlugin` trait. Host app implements this trait to enable device-specific shell commands.

## Overview

```rust
use fastshell::sdk::plugin::DevicePlugin;

struct MyPlugin;
impl DevicePlugin for MyPlugin {
    // Implement the methods you need. Unimplemented methods can return
    // Err("not implemented".into()) — the shell will return exit_code=1.
}

sdk.register_plugin(Box::new(MyPlugin));
```

## Camera

### `take_photo(&self, output_path: &str) -> Result<(), String>`
Shell command: `camera [/photo.jpg]`

Invoke the system camera to capture a photo. Save the resulting image file to `output_path` within the sandbox.

**Platform notes:**
- iOS: `AVCaptureSession` + `AVCapturePhotoOutput`
- Android: `CameraX` or `ACTION_IMAGE_CAPTURE` intent

### `take_screenshot(&self, output_path: &str) -> Result<(), String>`
Shell command: `screencapture [/screenshot.png]`

Capture a screenshot of the current screen.

**Platform notes:**
- iOS: `UIGraphicsImageRenderer` on key window
- Android: `MediaProjection` API

## Photo Library

### `pick_photo(&self, output_dir: &str, max_count: u32) -> Result<Vec<String>, String>`
Shell command: `photolib -n 5 -o /photos`

Open the system photo picker to select images. Copy selected files to `output_dir`. Returns list of saved file paths.

### `pick_video(&self, output_dir: &str) -> Result<String, String>`
Shell command: `photolib --video -o /videos`

Open the system photo picker to select a video. Returns the saved file path.

### `list_media(&self, media_type: &str, limit: u32) -> Result<Vec<MediaInfo>, String>`
Shell command: Used internally by `photolib`.

List media files (image/video) from the device library. Returns metadata.

**`MediaInfo` fields:** `name`, `path`, `size`, `mime`, `width`, `height`, `created`

## Audio

### `record_audio(&self, output_path: &str, duration_secs: u32) -> Result<(), String>`
Shell command: `record -d 10 -o /recording.wav`

Record audio from the microphone. Duration 0 means record until stopped externally.

**Platform notes:**
- iOS: `AVAudioRecorder`
- Android: `MediaRecorder`

### `play_audio(&self, path: &str) -> Result<(), String>`
Shell command: `play /music/song.mp3`

Play an audio file.

### `text_to_speech(&self, text: &str) -> Result<(), String>`
Shell command: `say "Hello world"`

Convert text to speech and play through the device speaker.

**Platform notes:**
- iOS: `AVSpeechSynthesizer`
- Android: `TextToSpeech`

### `speech_to_text(&self, input_path: &str) -> Result<String, String>`
Shell command: `speech /recording.wav`

Convert speech audio to text. Returns the recognized text.

**Platform notes:**
- iOS: `SFSpeechRecognizer`
- Android: `SpeechRecognizer`

## Contacts

### `list_contacts(&self, query: &str, limit: u32) -> Result<Vec<Contact>, String>`
Shell command: `contacts search "John" -n 20`

Query contacts by name/phone. Returns matching contacts.

**`Contact` fields:** `id`, `name`, `phones`, `emails`

### `get_contact(&self, contact_id: &str) -> Result<Contact, String>`
Shell command: `contacts get <id>`

Get full details for a specific contact.

**Platform notes:**
- iOS: `CNContactStore`
- Android: `ContactsContract`

## Location

### `get_location(&self) -> Result<Location, String>`
Shell command: `location`

Get the current device GPS position.

**`Location` fields:** `latitude`, `longitude`, `altitude`, `accuracy`, `speed`

**Platform notes:**
- iOS: `CLLocationManager`
- Android: `FusedLocationProviderClient`

## Clipboard

### `get_clipboard(&self) -> Result<String, String>`
Shell command: `clipboard` / `pbpaste`

Read text from the system clipboard.

**Platform notes:**
- iOS: `UIPasteboard.general.string`
- Android: `ClipboardManager`

### `set_clipboard(&self, text: &str) -> Result<(), String>`
Shell command: `clipboard set "text"` / `pbcopy "text"`

Write text to the system clipboard.

## Sensors

### `get_orientation(&self) -> Result<Orientation, String>`
Shell command: `sensor orientation`

Get device orientation (pitch, roll, yaw in radians).

### `get_motion(&self) -> Result<Motion, String>`
Shell command: `sensor motion`

Get accelerometer + gyroscope data (ax/ay/az in m/s², gx/gy/gz in rad/s).

### `get_ambient_light(&self) -> Result<f64, String>`
Shell command: `sensor light`

Get ambient light level in lux.

### `get_proximity(&self) -> Result<bool, String>`
Shell command: `sensor proximity`

Check if something is close to the proximity sensor.

### `list_sensors(&self) -> Result<Vec<SensorInfo>, String>`
Shell command: `sensor list`

List all available sensors on the device.

**Platform notes:**
- iOS: `CMMotionManager`
- Android: `SensorManager`

## Notifications

### `send_notification(&self, title: &str, body: &str, sound: bool) -> Result<(), String>`
Shell command: `notify "Title" "Body text" --sound`

Send a local system notification.

**Platform notes:**
- iOS: `UNUserNotificationCenter` (requires notification permission)
- Android: `NotificationManager` + `NotificationChannel`

## Share & Open

### `share_file(&self, path: &str, mime: &str) -> Result<(), String>`
Shell command: `share /photo.jpg --mime image/jpeg`

Open the system share sheet for a file.

### `share_text(&self, text: &str) -> Result<(), String>`
Shell command: `share --text "Check this out!"`

Open the system share sheet for text.

### `open_url(&self, url: &str) -> Result<(), String>`
Shell command: `open https://example.com` / `xdg-open file.pdf`

Open a URL or file with the system default handler.

**Platform notes:**
- iOS: `UIApplication.shared.open(url)`
- Android: `Intent(ACTION_VIEW)`

## Biometric

### `authenticate_biometric(&self, reason: &str) -> Result<bool, String>`
Shell command: `auth bio "Unlock fastshell"`

Prompt for fingerprint / face ID authentication. Returns true if authenticated.

**Platform notes:**
- iOS: `LAContext.evaluatePolicy`
- Android: `BiometricPrompt`

## Device State

### `get_battery(&self) -> Result<BatteryInfo, String>`
Shell command: `battery`

Get battery level (0.0-1.0), charging status, and power source.

**`BatteryInfo` fields:** `level`, `charging`, `source`

### `get_network_type(&self) -> Result<NetworkType, String>`
Shell command: `device network`

Get current network connection type.

**`NetworkType` fields:** `kind` (wifi/cellular/vpn/none), `connected`

### `set_brightness(&self, level: f64) -> Result<(), String>`
Shell command: `screen brightness 0.5`

Set screen brightness (0.0 = min, 1.0 = max).

### `keep_screen_on(&self, on: bool) -> Result<(), String>`
Shell command: `screen on` / `screen off`

Prevent screen from sleeping.

### `vibrate(&self, duration_ms: u32) -> Result<(), String>`
Shell command: `vibrate 500`

Trigger device vibration for the given duration.

### `get_device_info(&self) -> Result<DeviceInfo, String>`
Shell command: `device info`

Get device model, manufacturer, OS version, and screen dimensions.

**`DeviceInfo` fields:** `model`, `manufacturer`, `os_version`, `screen_width`, `screen_height`

## Permission Model

Each device command checks `PERMISSION_NEEDED:<type>:<resource>` before calling the plugin method. The first call returns exit_code=100, and the host app must call `sdk.set_permission()` to grant access. This applies to:

| Command | Permission resource |
|---------|-------------------|
| `camera` | `camera:photo` |
| `photolib` | `photolib:read` |
| `screencapture` | `screen:capture` |
| `record` | `microphone:record` |
| `speech` | `microphone:speech` |
| `contacts` | `contacts:read` |
| `location` | `location:gps` |

Commands not listed above (e.g., `clipboard`, `battery`, `open`) do not trigger permission checks.
