use std::fmt::Write;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::Manager;
use sysinfo::{Disks, Networks, ProcessesToUpdate, System, MINIMUM_CPU_UPDATE_INTERVAL};

pub struct SystemMonitor {
    sys: System,
    networks: Networks,
    disks: Disks,
    snapshot: SystemSnapshot,
    bluetooth_rx: Option<Receiver<BluetoothSnapshot>>,
    last_wifi: Instant,
    wifi_interval: Duration,
}

#[derive(Clone)]
pub struct SystemSnapshot {
    pub cpu_total_pct: f32,
    pub cpu_total_str: String,
    pub cpu_per_core: Vec<f32>,
    pub cpu_per_core_str: Vec<String>,
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_percent: f32,
    pub mem_label: String,
    pub disk_total: u64,
    pub disk_used: u64,
    pub disk_percent: f32,
    pub disk_label: String,
    pub net_rx_rate: String,
    pub net_tx_rate: String,
    pub processes: Vec<ProcessInfo>,
    pub wifi: WifiSnapshot,
    pub bluetooth: BluetoothSnapshot,
    pub gpus: Vec<GpuInfo>,
}

#[derive(Clone)]
pub struct ProcessInfo {
    pub pid: String,
    pub name: String,
    pub cpu_pct: f32,
    pub cpu_str: String,
    pub mem_str: String,
}

#[derive(Clone)]
pub struct WifiSnapshot {
    pub state: String,
    pub ssid: String,
    pub signal_label: String,
    pub signal_dbm_label: String,
}

#[derive(Clone)]
pub struct BluetoothSnapshot {
    pub state: String,
    pub devices_label: String,
}

#[derive(Clone)]
pub struct GpuInfo {
    pub label: String,
    pub usage: Option<f32>,
    pub usage_label: String,
}

impl SystemMonitor {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_cpu_usage();
        std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_cpu_usage();

        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();

        let snapshot = SystemSnapshot::new();
        let bluetooth_rx = spawn_bluetooth_worker();

        Self {
            sys,
            networks,
            disks,
            snapshot,
            bluetooth_rx,
            last_wifi: Instant::now() - Duration::from_secs(5),
            wifi_interval: Duration::from_secs(3),
        }
    }

    pub fn refresh(&mut self, elapsed: Duration) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        self.networks.refresh(true);
        self.disks.refresh(true);

        self.update_cpu();
        self.update_memory();
        self.update_disks();
        self.update_networks(elapsed);
        self.update_processes();
        self.update_wifi();
        self.update_bluetooth();
        self.update_gpu();
    }

    pub fn snapshot(&self) -> &SystemSnapshot {
        &self.snapshot
    }

    fn update_cpu(&mut self) {
        let cores = self.sys.cpus();
        self.snapshot.cpu_per_core.clear();
        self.snapshot.cpu_per_core.reserve(cores.len());
        let mut total = 0.0;
        for cpu in cores {
            let usage = cpu.cpu_usage();
            total += usage;
            self.snapshot.cpu_per_core.push(usage);
        }
        let core_count = self.snapshot.cpu_per_core.len().max(1) as f32;
        let avg = (total / core_count).clamp(0.0, 100.0);
        self.snapshot.cpu_total_pct = avg;
        self.snapshot.cpu_total_str = format_percent(avg);

        self.snapshot.cpu_per_core_str.clear();
        self.snapshot
            .cpu_per_core_str
            .reserve(self.snapshot.cpu_per_core.len());
        for (i, usage) in self.snapshot.cpu_per_core.iter().enumerate() {
            let mut line = String::new();
            let _ = write!(line, "C{:02} {:>5.1}%", i, usage.clamp(0.0, 100.0));
            self.snapshot.cpu_per_core_str.push(line);
        }
    }

    fn update_memory(&mut self) {
        self.snapshot.mem_total = self.sys.total_memory();
        self.snapshot.mem_used = self.sys.used_memory();
        let pct = if self.snapshot.mem_total > 0 {
            (self.snapshot.mem_used as f32 / self.snapshot.mem_total as f32) * 100.0
        } else {
            0.0
        };
        self.snapshot.mem_percent = pct.clamp(0.0, 100.0);
        let used = format_bytes(self.snapshot.mem_used);
        let total = format_bytes(self.snapshot.mem_total);
        self.snapshot.mem_label = format!("{} / {}", used, total);
    }

    fn update_disks(&mut self) {
        let mut total = 0u64;
        let mut available = 0u64;
        for disk in &self.disks {
            total = total.saturating_add(disk.total_space());
            available = available.saturating_add(disk.available_space());
        }
        let used = total.saturating_sub(available);
        self.snapshot.disk_total = total;
        self.snapshot.disk_used = used;
        let pct = if total > 0 {
            (used as f32 / total as f32) * 100.0
        } else {
            0.0
        };
        self.snapshot.disk_percent = pct.clamp(0.0, 100.0);
        let used_str = format_bytes(used);
        let total_str = format_bytes(total);
        self.snapshot.disk_label = format!("{} / {}", used_str, total_str);
    }

    fn update_networks(&mut self, elapsed: Duration) {
        let mut rx = 0u64;
        let mut tx = 0u64;
        for (_, data) in &self.networks {
            rx = rx.saturating_add(data.received());
            tx = tx.saturating_add(data.transmitted());
        }
        let secs = elapsed.as_secs_f64().max(0.001);
        let rx_rate = rx as f64 / secs;
        let tx_rate = tx as f64 / secs;
        self.snapshot.net_rx_rate = format_rate(rx_rate);
        self.snapshot.net_tx_rate = format_rate(tx_rate);
    }

    fn update_processes(&mut self) {
        self.snapshot.processes.clear();
        self.snapshot.processes.reserve(5);
        let total_mem = self.snapshot.mem_total.max(1) as f32;
        let cores = self.snapshot.cpu_per_core.len().max(1) as f32;
        for (pid, process) in self.sys.processes() {
            let cpu = process.cpu_usage();
            let cpu_norm = (cpu / cores).clamp(0.0, 100.0);
            let mem = process.memory() as f32;
            let mem_pct = ((mem / total_mem) * 100.0).clamp(0.0, 100.0);
            let info = ProcessInfo {
                pid: pid.as_u32().to_string(),
                name: process.name().to_string_lossy().into_owned(),
                cpu_pct: cpu_norm,
                cpu_str: format_percent(cpu_norm),
                mem_str: format_percent(mem_pct),
            };
            insert_top(&mut self.snapshot.processes, info, 5);
        }
    }

    fn update_wifi(&mut self) {
        if self.last_wifi.elapsed() < self.wifi_interval {
            return;
        }
        self.last_wifi = Instant::now();
        match wifi_scan::scan() {
            Ok(list) if !list.is_empty() => {
                let best = list
                    .iter()
                    .max_by_key(|w| w.signal_level)
                    .unwrap();
                let ssid = if best.ssid.is_empty() {
                    "<hidden>".to_string()
                } else {
                    best.ssid.clone()
                };
                let signal_dbm = best.signal_level;
                let signal_percent = dbm_to_percent(signal_dbm);
                self.snapshot.wifi = WifiSnapshot {
                    state: "Available".to_string(),
                    ssid,
                    signal_label: format!("{}%", signal_percent),
                    signal_dbm_label: format!("({} dBm)", signal_dbm),
                };
            }
            _ => {
                self.snapshot.wifi = WifiSnapshot {
                    state: "No WiFi".to_string(),
                    ssid: "N/A".to_string(),
                    signal_label: "N/A".to_string(),
                    signal_dbm_label: "".to_string(),
                };
            }
        }
    }

    fn update_bluetooth(&mut self) {
        let Some(rx) = &self.bluetooth_rx else {
            return;
        };
        while let Ok(snapshot) = rx.try_recv() {
            self.snapshot.bluetooth = snapshot;
        }
    }

    fn update_gpu(&mut self) {
        self.snapshot.gpus.clear();
        for (index, usage) in probe_gpu_usages().into_iter().enumerate() {
            if index >= 2 {
                break;
            }
            let label = format!("GPU {}", index);
            let usage_label = usage
                .map(|v| format!("{:.0}%", v.clamp(0.0, 100.0)))
                .unwrap_or_else(|| "N/A".to_string());
            self.snapshot.gpus.push(GpuInfo {
                label,
                usage,
                usage_label,
            });
        }
        while self.snapshot.gpus.len() < 2 {
            let label = format!("GPU {}", self.snapshot.gpus.len());
            self.snapshot.gpus.push(GpuInfo {
                label,
                usage: None,
                usage_label: "N/A".to_string(),
            });
        }
    }
}

impl SystemSnapshot {
    fn new() -> Self {
        let mut gpus = Vec::with_capacity(2);
        gpus.push(GpuInfo {
            label: "GPU 0".to_string(),
            usage: None,
            usage_label: "N/A".to_string(),
        });
        gpus.push(GpuInfo {
            label: "GPU 1".to_string(),
            usage: None,
            usage_label: "N/A".to_string(),
        });
        Self {
            cpu_total_pct: 0.0,
            cpu_total_str: "0.0%".to_string(),
            cpu_per_core: Vec::new(),
            cpu_per_core_str: Vec::new(),
            mem_total: 0,
            mem_used: 0,
            mem_percent: 0.0,
            mem_label: "0 B / 0 B".to_string(),
            disk_total: 0,
            disk_used: 0,
            disk_percent: 0.0,
            disk_label: "0 B / 0 B".to_string(),
            net_rx_rate: "0 B/s".to_string(),
            net_tx_rate: "0 B/s".to_string(),
            processes: Vec::with_capacity(5),
            wifi: WifiSnapshot {
                state: "No WiFi".to_string(),
                ssid: "N/A".to_string(),
                signal_label: "N/A".to_string(),
                signal_dbm_label: "".to_string(),
            },
            bluetooth: BluetoothSnapshot {
                state: "No Bluetooth".to_string(),
                devices_label: "None".to_string(),
            },
            gpus,
        }
    }
}

fn insert_top(list: &mut Vec<ProcessInfo>, info: ProcessInfo, k: usize) {
    let pos = list
        .iter()
        .position(|p| info.cpu_pct > p.cpu_pct)
        .unwrap_or(list.len());
    if pos < k {
        list.insert(pos, info);
        if list.len() > k {
            list.pop();
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = units[0];
    for next in &units[1..] {
        if value >= 1024.0 {
            value /= 1024.0;
            unit = next;
        }
    }
    if unit == "B" {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", value, unit)
    }
}

fn format_rate(bytes_per_sec: f64) -> String {
    let units = ["B/s", "KB/s", "MB/s", "GB/s", "TB/s"];
    let mut value = bytes_per_sec;
    let mut unit = units[0];
    for next in &units[1..] {
        if value >= 1024.0 {
            value /= 1024.0;
            unit = next;
        }
    }
    if unit == "B/s" {
        format!("{:.0} B/s", value)
    } else {
        format!("{:.1} {}", value, unit)
    }
}

fn format_percent(value: f32) -> String {
    format!("{:.1}%", value)
}

fn dbm_to_percent(dbm: i32) -> u8 {
    let clamped = dbm.clamp(-90, -30);
    let pct = ((clamped + 90) as f32 / 60.0) * 100.0;
    pct.round().clamp(0.0, 100.0) as u8
}

fn spawn_bluetooth_worker() -> Option<Receiver<BluetoothSnapshot>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_time()
            .build();
        let Ok(rt) = rt else {
            return;
        };
        rt.block_on(async move {
            bluetooth_task(tx).await;
        });
    });
    Some(rx)
}

async fn bluetooth_task(tx: Sender<BluetoothSnapshot>) {
    loop {
        let manager = match Manager::new().await {
            Ok(m) => m,
            Err(_) => {
                let _ = tx.send(BluetoothSnapshot {
                    state: "No Bluetooth".to_string(),
                    devices_label: "None".to_string(),
                });
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        let adapters = manager.adapters().await.unwrap_or_default();
        if adapters.is_empty() {
            let _ = tx.send(BluetoothSnapshot {
                state: "No Bluetooth".to_string(),
                devices_label: "None".to_string(),
            });
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        }
        let adapter = adapters[0].clone();
        let _ = adapter.start_scan(ScanFilter::default()).await;
        loop {
            let state = match adapter.adapter_state().await {
                Ok(s) => format!("{s:?}"),
                Err(_) => "Unknown".to_string(),
            };
            let peripherals = adapter.peripherals().await.unwrap_or_default();
            let mut devices = Vec::new();
            for p in peripherals {
                let connected = p.is_connected().await.unwrap_or(false);
                if connected {
                    let name = match p.properties().await {
                        Ok(Some(props)) => props
                            .local_name
                            .unwrap_or_else(|| format!("{:?}", p.id())),
                        _ => format!("{:?}", p.id()),
                    };
                    devices.push(name);
                }
            }
            devices.sort();
            devices.truncate(5);
            let devices_label = if devices.is_empty() {
                "None".to_string()
            } else {
                devices.join(", ")
            };
            let snapshot = BluetoothSnapshot {
                state,
                devices_label,
            };
            if tx.send(snapshot).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

#[cfg(target_os = "linux")]
fn probe_gpu_usages() -> Vec<Option<f32>> {
    let mut values = Vec::new();
    let entries = match std::fs::read_dir("/sys/class/drm") {
        Ok(entries) => entries,
        Err(_) => return values,
    };
    for entry in entries.flatten() {
        let path = entry.path().join("device").join("gpu_busy_percent");
        if let Ok(data) = std::fs::read_to_string(path) {
            let value = data.trim().parse::<f32>().ok();
            values.push(value);
        }
    }
    values
}

#[cfg(not(target_os = "linux"))]
fn probe_gpu_usages() -> Vec<Option<f32>> {
    Vec::new()
}
