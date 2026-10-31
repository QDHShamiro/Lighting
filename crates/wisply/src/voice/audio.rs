use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, EDataFlow,
    IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    WAVEFORMATEX, eCapture, eRender,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};

pub const RATE: u32 = 48_000;
pub const BLOCK: usize = 960;
const FLOAT_TAG: u16 = 3;
const BUFFER_HNS: i64 = 2_000_000;
const TICK: Duration = Duration::from_millis(5);
const SILENCE_LAG: usize = 2 * BLOCK;

pub enum Kind {
    Speaker,
    Mic,
    Loopback,
}

fn format(channels: u16) -> WAVEFORMATEX {
    let align = channels * 4;
    WAVEFORMATEX {
        wFormatTag: FLOAT_TAG,
        nChannels: channels,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * u32::from(align),
        nBlockAlign: align,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

fn name_of(dev: &IMMDevice) -> String {
    unsafe {
        let Ok(store) = dev.OpenPropertyStore(STGM_READ) else { return String::new() };
        let Ok(value) = store.GetValue(&PKEY_Device_FriendlyName) else { return String::new() };
        let Ok(text) = PropVariantToStringAlloc(&raw const value) else { return String::new() };
        let out = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as *const _));
        out
    }
}

fn flow(kind: &Kind) -> EDataFlow {
    if matches!(kind, Kind::Mic) { eCapture } else { eRender }
}

pub fn names(kind: &Kind) -> Vec<String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return Vec::new();
        };
        let Ok(all) = en.EnumAudioEndpoints(flow(kind), DEVICE_STATE_ACTIVE) else { return Vec::new() };
        let n = all.GetCount().unwrap_or(0);
        (0..n).filter_map(|i| all.Item(i).ok()).map(|d| name_of(&d)).collect()
    }
}

fn open(kind: &Kind, want: &str) -> Result<(IMMDevice, String), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| format!("no audio system: {e}"))?;
        let all = en.EnumAudioEndpoints(flow(kind), DEVICE_STATE_ACTIVE).map_err(|e| e.to_string())?;
        let low = want.to_lowercase();
        for i in 0..all.GetCount().unwrap_or(0) {
            let Ok(dev) = all.Item(i) else { continue };
            let name = name_of(&dev);
            if name.to_lowercase().contains(&low) {
                return Ok((dev, name));
            }
        }
    }
    let what = match kind {
        Kind::Speaker => "speaker",
        Kind::Mic => "microphone",
        Kind::Loopback => "loop",
    };
    Err(format!("no {what} device matching '{want}'"))
}

fn client(dev: &IMMDevice, kind: &Kind, channels: u16) -> Result<IAudioClient, String> {
    let fmt = format(channels);
    let mut flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    if matches!(kind, Kind::Loopback) {
        flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }
    unsafe {
        let c: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        c.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_HNS, 0, &raw const fmt, None)
            .map_err(|e| e.to_string())?;
        Ok(c)
    }
}

pub fn record(kind: &Kind, want: &str, channels: u16, mut take: impl FnMut(&[f32]) -> bool) -> Result<(), String> {
    let (dev, _) = open(kind, want)?;
    let c = client(&dev, kind, channels)?;
    let ch = usize::from(channels);
    let block = BLOCK * ch;
    let mut pending: Vec<f32> = Vec::with_capacity(block * 4);
    unsafe {
        let cap: IAudioCaptureClient = c.GetService().map_err(|e| e.to_string())?;
        c.Start().map_err(|e| e.to_string())?;
        let t0 = Instant::now();
        let mut given = 0usize;
        loop {
            while cap.GetNextPacketSize().unwrap_or(0) > 0 {
                let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                if cap.GetBuffer(&raw mut data, &raw mut frames, &raw mut flags, None, None).is_err() {
                    break;
                }
                let n = frames as usize * ch;
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    pending.extend(std::iter::repeat_n(0.0, n));
                } else {
                    pending.extend_from_slice(std::slice::from_raw_parts(data.cast::<f32>(), n));
                }
                let _ = cap.ReleaseBuffer(frames);
            }
            let due = (t0.elapsed().as_secs_f64() * f64::from(RATE)) as usize;
            if matches!(kind, Kind::Loopback) && pending.is_empty() && due > given + SILENCE_LAG {
                pending.extend(std::iter::repeat_n(0.0, (due - given - BLOCK) * ch));
            }
            while pending.len() >= block {
                given += BLOCK;
                if !take(&pending[..block]) {
                    let _ = c.Stop();
                    return Ok(());
                }
                pending.drain(..block);
            }
            thread::sleep(TICK);
        }
    }
}

pub struct Player {
    c: IAudioClient,
    out: IAudioRenderClient,
    size: u32,
    ch: usize,
}

impl Player {
    pub fn open(want: &str, channels: u16) -> Result<Player, String> {
        let (dev, _) = open(&Kind::Speaker, want)?;
        let c = client(&dev, &Kind::Speaker, channels)?;
        unsafe {
            let out: IAudioRenderClient = c.GetService().map_err(|e| e.to_string())?;
            let size = c.GetBufferSize().map_err(|e| e.to_string())?;
            c.Start().map_err(|e| e.to_string())?;
            Ok(Player { c, out, size, ch: usize::from(channels) })
        }
    }

    pub fn play(&self, mut samples: &[f32]) {
        while samples.len() >= self.ch {
            let used = unsafe { self.c.GetCurrentPadding() }.unwrap_or(self.size);
            let room = (self.size - used.min(self.size)) as usize;
            if room == 0 {
                thread::sleep(TICK);
                continue;
            }
            let frames = room.min(samples.len() / self.ch);
            unsafe {
                let Ok(buf) = self.out.GetBuffer(frames as u32) else { return };
                std::ptr::copy_nonoverlapping(samples.as_ptr(), buf.cast::<f32>(), frames * self.ch);
                let _ = self.out.ReleaseBuffer(frames as u32, 0);
            }
            samples = &samples[frames * self.ch..];
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = unsafe { self.c.Stop() };
    }
}
