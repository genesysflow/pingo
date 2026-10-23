//! Explicit, local diagnostic runs are enabled by environment on debug builds only.
use crate::video::{VideoHub, now_ms};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::BufReader,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{
    Manager,
    ipc::{Channel, Response},
};

#[tauri::command]
pub fn desktop_probe() -> Value {
    if !cfg!(debug_assertions) {
        return Value::Null;
    }
    json!({"video":std::env::var("PINGO_DESKTOP_PROBE").ok().as_deref()==Some("1"),"smoke":std::env::var("PINGO_DESKTOP_SMOKE").ok().and_then(|v|serde_json::from_str::<Value>(&v).ok())})
}
#[tauri::command]
pub fn desktop_probe_report(app: tauri::AppHandle, report: Value) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("Debug diagnostics only".into());
    }
    let path = std::env::var("PINGO_DESKTOP_REPORT")
        .map_err(|_| "No diagnostic output path configured")?;
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}
fn cpu_tree() -> f64 {
    let mut processes = HashMap::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            let Some((_, rest)) = stat.rsplit_once(')') else {
                continue;
            };
            let fields: Vec<_> = rest.split_whitespace().collect();
            if fields.len() > 12 {
                processes.insert(
                    pid,
                    (
                        fields[1].parse::<u32>().unwrap_or(0),
                        fields[11].parse::<f64>().unwrap_or(0.0)
                            + fields[12].parse::<f64>().unwrap_or(0.0),
                    ),
                );
            }
        }
    }
    processes
        .iter()
        .filter(|(pid, _)| {
            let mut parent = **pid;
            for _ in 0..32 {
                if parent == std::process::id() {
                    return true;
                }
                parent = processes.get(&parent).map_or(0, |p| p.0);
                if parent == 0 {
                    break;
                }
            }
            false
        })
        .map(|(_, p)| p.1 / 100.0)
        .sum()
}
#[tauri::command]
pub async fn video_spike(
    app: tauri::AppHandle,
    mode: String,
    fps: u32,
    channel: Channel<Response>,
) -> Result<Value, String> {
    if !cfg!(debug_assertions)
        || std::env::var("PINGO_DESKTOP_PROBE").ok().as_deref() != Some("1")
        || ![30, 60].contains(&fps)
        || ![
            "h264",
            "vp8",
            "jpeg_h264",
            "jpeg_vp8",
            "encodedH264",
            "encodedVp8",
        ]
        .contains(&mode.as_str())
    {
        return Err("Video spike not enabled or invalid mode".into());
    }
    let directory =
        std::env::var("PINGO_VIDEO_FIXTURES").map_err(|_| "Missing fixture directory")?;
    let hub = app.state::<Arc<VideoHub>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let key = format!("spike:{mode}:{fps}");
        if mode.starts_with("encoded") {
            let path = std::path::Path::new(&directory).join(if mode=="encodedH264" { "first.h264" } else { "first.vp8" });
            channel.send(Response::new(std::fs::read(path).map_err(|e|e.to_string())?)).map_err(|e|e.to_string())?;
            return Ok(json!({"sent":1}));
        }
        let jpeg=mode.starts_with("jpeg_");
        let codec=mode.strip_prefix("jpeg_").unwrap_or(&mode);
        hub.subscribe(key.clone(),channel)?;
        let path = std::path::Path::new(&directory).join(format!("{fps}.{}", if codec=="h264" {"h264"} else {"ivf"}));
        let start_cpu = cpu_tree(); let start = Instant::now();
        let mut command = Command::new("ffmpeg");
        command.args(["-hide_banner","-loglevel","error","-threads","2","-re","-f",if codec=="h264" {"h264"} else {"ivf"},"-i"]).arg(path)
            ;
        command.args(["-an","-threads","2"]);
        if jpeg { command.args(["-q:v","3","-c:v","mjpeg"]); } else { command.args(["-pix_fmt","rgba","-c:v","pam"]); }
        let mut child=command.args(["-f","image2pipe","-flush_packets","1","pipe:1"]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e|e.to_string())?;
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let (mut frames,mut sent) = (0,0);
        loop {
            if jpeg {
                let Some(pixels)=crate::video::read_jpeg(&mut output).map_err(|e|e.to_string())? else { break; };
                frames+=1; if hub.jpeg(&key,&pixels,now_ms()) { sent+=1; }
            } else {
                let Some((width,height,pixels))=crate::video::read_pam(&mut output).map_err(|e|e.to_string())? else { break; };
                frames+=1; if hub.raw(&key,width,height,&pixels,now_ms()) { sent+=1; }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        let seconds=start.elapsed().as_secs_f64(); let cpu=(cpu_tree()-start_cpu)/seconds*100.0;
        let status=child.wait().map_err(|e|e.to_string())?; hub.unsubscribe(&key);
        if !status.success() { return Err(format!("FFmpeg spike failed: {status}")); }
        Ok(json!({"mode":mode,"fps":fps,"frames":frames,"sent":sent,"dropped":frames-sent,"seconds":seconds,"cpuPercentOneCore":cpu,"cpuClockTicksPerSecond":100}))
    }).await.map_err(|e|e.to_string())?
}
