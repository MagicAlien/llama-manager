//! `core/env_probe.rs` — `docs/TASKS.md` T-010.
//!
//! GPU detection through the `NvmlProvider` seam (`docs/CONTRACTS.md` §6),
//! plus system RAM, free disk space and the OS build string, assembled
//! into the `HealthCheck` list and `EnvironmentReport` `docs/CONTRACTS.md`
//! §1's Environment block already defines (transcribed into
//! `core/types.rs` by T-002; this module is the first to *construct*
//! those types outside a test).
//!
//! `NvmlProvider` is declared here rather than in `core/types.rs`:
//! `docs/CONTRACTS.md` §6 titles it "Provider traits", a section distinct
//! from §1's plain data types, and nothing constructed by `core/types.rs`
//! today is a trait. The seam lives beside the code that depends on it,
//! the same way `db/mod.rs` keeps its own `AppError`-mapping helpers
//! rather than relocating them to `core/types.rs`.
//!
//! **Two engineering decisions this file makes that no document in this
//! repo settles, recorded here and in `PROGRESS.md` F-004 rather than
//! picked silently:**
//!
//! - `driver_ok` and `cuda13_ok`'s numeric floors. Neither
//!   `docs/CONTRACTS.md`, `docs/LLAMACPP.md` nor `PLAN.md` §2.2 states a
//!   minimum NVIDIA driver version anywhere — `PLAN.md` §2.2 explains
//!   *why* CUDA 13 matters (an architecture the binary was not compiled
//!   for silently falls back to CPU) but not what driver a CUDA 13 build
//!   needs. This is answerable from NVIDIA's own public documentation,
//!   not a genuine gap, so it is not raised as a Discrepancy: see
//!   `MIN_DRIVER_MAJOR_CUDA13` and `MIN_DRIVER_MAJOR_GENERAL` below for
//!   the citation.
//! - `disk_space`'s Warn/Fail thresholds have no source to cite at all —
//!   an ordinary implementation choice, not a project fact, and reversible
//!   without affecting any other task.
//!
//! T-010 left `#[allow(dead_code)]` here because nothing rooted this
//! module from the binary's own entry point yet. T-011 (docs/TASKS.md
//! T-011) wires `ipc::probe_environment` into `main.rs`'s
//! `invoke_handler`, which calls `RealNvmlProvider` and `probe` directly —
//! the allow is removed accordingly.

use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::OnceLock;

use crate::core::types::{AppError, CheckStatus, EnvironmentReport, GpuInfo, GpuTelemetry, HealthCheck};

// ─── Provider traits (`docs/CONTRACTS.md` §6) ──────────────────────────

/// GPU access. Every task that reads GPU state goes through this, which is
/// why a GPU-less machine can take T-010 (and, later, T-060/T-061) to
/// green. A failure degrades to a typed error; it never panics.
pub trait NvmlProvider: Send + Sync {
    fn device_count(&self) -> Result<u32, AppError>;
    fn device_info(&self, index: u32) -> Result<GpuInfo, AppError>;
    // Not called by `probe()` or anything else yet — reserved for a
    // future GPU telemetry view (T-060/T-061, named above). Real
    // `cargo build` output (T-011 PR review) confirmed this is now the
    // only dead-code warning left once the module-wide `#![allow(dead_code)]`
    // T-010 added was removed; scoped here rather than reintroducing
    // that module-wide allow, since everything else in this module now
    // has a real caller.
    #[allow(dead_code)]
    fn device_telemetry(&self, index: u32) -> Result<GpuTelemetry, AppError>;
    fn driver_version(&self) -> Result<String, AppError>;
}

// ─── Tunable floors and thresholds ─────────────────────────────────────

/// Minimum NVIDIA driver branch this app treats as viable at all.
///
/// Sourced from NVIDIA's own "Driver Range for Minor Version
/// Compatibility" table (CUDA Toolkit 13.0 Update 3 Release Notes, §2.2
/// "CUDA Driver"): 13.x needs >= 580; 12.x needs >= 525 and < 580; 11.x
/// needs >= 450 and < 525. `PLAN.md` §2.2 frames CUDA 12 as this project's
/// oldest real fallback for non-Blackwell hardware ("Non-Blackwell NVIDIA
/// prefers CUDA 13 and may fall back to CUDA 12") and names nothing older
/// — so the CUDA 12.x floor is the general viability floor here, not an
/// arbitrary number.
/// Source: <https://docs.nvidia.com/cuda/archive/13.0.3/cuda-toolkit-release-notes/index.html>
/// (Table: "CUDA Toolkit and Corresponding Driver Versions" / "Driver
/// Range for Minor Version Compatibility"). Recorded as `PROGRESS.md` F-004.
const MIN_DRIVER_MAJOR_GENERAL: u32 = 525;

/// Minimum NVIDIA driver branch required to run a CUDA 13 build at all
/// (`PLAN.md` §2.2 — Blackwell requires the CUDA 13 asset or nothing, and
/// a driver below this cannot run one regardless of GPU). Same source as
/// `MIN_DRIVER_MAJOR_GENERAL`, same F-004 note: CUDA 13.0 GA's published
/// floor is driver >= 580.65.06, and the "Driver Range for Minor Version
/// Compatibility" table states the 13.x floor as driver >= 580 without
/// distinguishing Windows from Linux — NVIDIA's driver version numbering
/// has been unified across both platforms since well before the 580
/// branch, so the same number applies here on Windows.
const MIN_DRIVER_MAJOR_CUDA13: u32 = 580;

/// Below this, `disk_space` fails outright: not even a fresh SQLite
/// database and a day of logs are guaranteed to fit. Not sourced from any
/// document — an implementation choice, easy to revisit, and nothing else
/// depends on the exact number.
const DISK_FAIL_BELOW_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB

/// Below this (and at or above the fail floor), `disk_space` warns:
/// comfortably enough for the app itself, not necessarily for a large
/// model download (`docs/TASKS.md` T-022). Same caveat as the fail floor.
const DISK_WARN_BELOW_BYTES: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB

/// `ServerConfig::listen_port` (`docs/CONTRACTS.md` ~line 397) defaults to
/// 8080. `ServerConfig` has no `Default` impl yet — T-050 is what wires
/// real persisted settings — so `probe` takes the port as a parameter
/// rather than reaching for a type this module has no real instance of;
/// callers without a configured value yet should pass this constant.
pub const DEFAULT_LISTEN_PORT: u16 = 8080;

// ─── Entry point ────────────────────────────────────────────────────────

/// Builds a complete `EnvironmentReport`. Never panics and never returns
/// an error itself: every internal failure (an absent GPU, a failed NVML
/// call, an unreadable disk or registry value) degrades into a `HealthCheck`
/// entry describing it, which is what `docs/TASKS.md` T-010's acceptance
/// criteria asks for directly ("No-GPU yields `gpu_present = Fail` ... and
/// no panic", "NVML failing mid-call is handled, not propagated as a
/// panic").
pub fn probe(provider: &dyn NvmlProvider, listen_port: u16) -> EnvironmentReport {
    let (gpus, mut checks) = probe_gpus(provider);
    checks.push(disk_space_check(free_disk_bytes()));
    checks.push(endpoint_bindable(listen_port));

    EnvironmentReport {
        gpus,
        system_ram_bytes: system_ram_bytes(),
        free_disk_bytes: free_disk_bytes(),
        os_build: os_build_string(),
        checks,
    }
}

// ─── GPU checks ─────────────────────────────────────────────────────────

fn probe_gpus(provider: &dyn NvmlProvider) -> (Vec<GpuInfo>, Vec<HealthCheck>) {
    // A typed error here (no driver, NVML not installed, `Nvml::init`
    // failed) is indistinguishable from "no GPU" for reporting purposes —
    // either way there is nothing to enumerate, and the failure must not
    // propagate as a panic. `unwrap_or_default()` rather than a `match`
    // per `cargo clippy`'s own `manual_unwrap_or_default` finding (real
    // gate run, first fixup — PROGRESS.md).
    let count = provider.device_count().unwrap_or_default();

    if count == 0 {
        return (Vec::new(), no_gpu_checks());
    }

    let mut gpus = Vec::with_capacity(count as usize);
    for index in 0..count {
        match provider.device_info(index) {
            Ok(info) => gpus.push(info),
            // One device failing mid-call does not fail the whole probe —
            // the acceptance criterion is exactly this: "NVML failing
            // mid-call is handled, not propagated as a panic."
            Err(_) => continue,
        }
    }

    if gpus.is_empty() {
        // `device_count` answered > 0 and every `device_info` call then
        // failed — NVML is present but unusable. Reported the same way as
        // no GPU at all: there is still nothing to show.
        return (Vec::new(), no_gpu_checks());
    }

    let driver = provider.driver_version().ok();
    let checks = driver_checks(&gpus, driver.as_deref());
    (gpus, checks)
}

fn no_gpu_checks() -> Vec<HealthCheck> {
    let remediation = || {
        Some(
            "Install a supported NVIDIA GPU and driver, or continue on Vulkan/CPU \
             (expect much lower performance, PLAN.md §2.2)."
                .to_string(),
        )
    };
    vec![
        HealthCheck {
            id: "gpu_present".to_string(),
            status: CheckStatus::Fail,
            message: "No NVIDIA GPU was detected.".to_string(),
            remediation: remediation(),
        },
        HealthCheck {
            id: "driver_ok".to_string(),
            status: CheckStatus::Fail,
            message: "No NVIDIA driver was detected.".to_string(),
            remediation: remediation(),
        },
        HealthCheck {
            id: "cuda13_ok".to_string(),
            status: CheckStatus::Fail,
            message: "No NVIDIA driver was detected; CUDA 13 support cannot be confirmed.".to_string(),
            remediation: remediation(),
        },
    ]
}

/// Parses the leading dot-separated major component of an NVML driver
/// version string (`"580.65.06"` -> `Some(580)`). Never panics: a string
/// that does not start with an integer yields `None`, which every caller
/// here treats as "cannot confirm the floor is met", the same as an NVML
/// error would be — not as a crash.
fn driver_major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse::<u32>().ok()
}

fn driver_checks(gpus: &[GpuInfo], system_driver: Option<&str>) -> Vec<HealthCheck> {
    let gpu_present = HealthCheck {
        id: "gpu_present".to_string(),
        status: CheckStatus::Pass,
        message: format!("{} NVIDIA GPU(s) detected.", gpus.len()),
        remediation: None,
    };

    // Prefer the provider's own system-level call; fall back to the first
    // GPU's own `driver_version` field (`GpuInfo`, `docs/CONTRACTS.md` §1)
    // if that call failed but per-device info carried the value anyway.
    // Either way, an absent value degrades to "cannot confirm", not a panic.
    let driver_string: Option<String> = system_driver
        .map(str::to_string)
        .or_else(|| gpus.first().map(|g| g.driver_version.clone()));
    let major = driver_string.as_deref().and_then(driver_major);
    let shown = driver_string.as_deref().unwrap_or("unknown");

    let driver_ok = match major {
        Some(m) if m >= MIN_DRIVER_MAJOR_GENERAL => HealthCheck {
            id: "driver_ok".to_string(),
            status: CheckStatus::Pass,
            message: format!("NVIDIA driver {shown} detected."),
            remediation: None,
        },
        Some(_) => HealthCheck {
            id: "driver_ok".to_string(),
            status: CheckStatus::Fail,
            message: format!("NVIDIA driver {shown} is too old (need {MIN_DRIVER_MAJOR_GENERAL}+)."),
            remediation: Some("Update your NVIDIA driver from nvidia.com/drivers.".to_string()),
        },
        None => HealthCheck {
            id: "driver_ok".to_string(),
            status: CheckStatus::Fail,
            message: "NVIDIA driver version could not be read.".to_string(),
            remediation: Some("Update your NVIDIA driver from nvidia.com/drivers.".to_string()),
        },
    };

    let cuda13_ok = match major {
        Some(m) if m >= MIN_DRIVER_MAJOR_CUDA13 => HealthCheck {
            id: "cuda13_ok".to_string(),
            status: CheckStatus::Pass,
            message: "NVIDIA driver supports CUDA 13.".to_string(),
            remediation: None,
        },
        Some(_) => HealthCheck {
            id: "cuda13_ok".to_string(),
            status: CheckStatus::Fail,
            message: format!("NVIDIA driver {shown} does not support CUDA 13 (need {MIN_DRIVER_MAJOR_CUDA13}+)."),
            remediation: Some("Update your NVIDIA driver from nvidia.com/drivers.".to_string()),
        },
        None => HealthCheck {
            id: "cuda13_ok".to_string(),
            status: CheckStatus::Fail,
            message: "NVIDIA driver version could not be read; CUDA 13 support cannot be confirmed.".to_string(),
            remediation: Some("Update your NVIDIA driver from nvidia.com/drivers.".to_string()),
        },
    };

    vec![gpu_present, driver_ok, cuda13_ok]
}

// ─── Disk space ─────────────────────────────────────────────────────────

/// Pure decision function, factored out of `disk_space_check` so the
/// Warn/Fail/remediation logic is testable without depending on the real
/// disk state of whatever machine runs the test — `free_disk_bytes` below
/// is not behind a seam (`docs/CONTRACTS.md` §6 names only `NvmlProvider`
/// and `RegistryProvider`), so this is the only way to exercise the Warn
/// and Fail branches deterministically.
fn disk_status(free_bytes: u64) -> HealthCheck {
    let gib = |b: u64| b / (1024 * 1024 * 1024);
    if free_bytes < DISK_FAIL_BELOW_BYTES {
        HealthCheck {
            id: "disk_space".to_string(),
            status: CheckStatus::Fail,
            message: format!("Only {} GiB free.", gib(free_bytes)),
            remediation: Some("Free up disk space before installing a runtime or model.".to_string()),
        }
    } else if free_bytes < DISK_WARN_BELOW_BYTES {
        HealthCheck {
            id: "disk_space".to_string(),
            status: CheckStatus::Warn,
            message: format!("Only {} GiB free.", gib(free_bytes)),
            remediation: Some(
                "Large models can be tens of gigabytes; consider freeing up space.".to_string(),
            ),
        }
    } else {
        HealthCheck {
            id: "disk_space".to_string(),
            status: CheckStatus::Pass,
            message: format!("{} GiB free.", gib(free_bytes)),
            remediation: None,
        }
    }
}

fn disk_space_check(free_bytes: u64) -> HealthCheck {
    disk_status(free_bytes)
}

// ─── Endpoint bindability ───────────────────────────────────────────────

/// `docs/TASKS.md` T-010: "reports whether the configured listen port can
/// be bound, without holding it." Binds, then immediately drops the
/// listener — the port is never held past this check.
fn endpoint_bindable(port: u16) -> HealthCheck {
    match TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), port)) {
        Ok(listener) => {
            drop(listener);
            HealthCheck {
                id: "endpoint_bindable".to_string(),
                status: CheckStatus::Pass,
                message: format!("Port {port} is free."),
                remediation: None,
            }
        }
        Err(err) => HealthCheck {
            id: "endpoint_bindable".to_string(),
            status: CheckStatus::Fail,
            message: format!("Port {port} could not be bound: {err}."),
            remediation: Some(format!(
                "Close whatever else on this machine is using port {port}, or choose a \
                 different listen port once settings are available."
            )),
        },
    }
}

// ─── System information (Windows-only) ─────────────────────────────────
//
// `AGENTS.md` targets Windows 11 exclusively, the same assumption
// `main.rs`'s `LOCALAPPDATA` lookup already makes without a `cfg` gate —
// so these are written the same way, with no non-Windows fallback path.
//
// `windows` is already on `PLAN.md` §3's approved crate list, scoped in
// that list's own parenthetical to "DPAPI, junctions" — neither of which
// this task uses. Justified here the same way `chrono` was in T-002
// (`PROGRESS.md` D-008, `AGENTS.md` §4): the crate itself is already
// approved, this is a new feature area of it, and there is no `std` API
// for physical RAM, drive free space, or the OS build string on Windows.
//
// **Unverified against a real compile.** This session has neither a Rust
// toolchain nor a Windows machine (see `PROGRESS.md`); this block —
// `GlobalMemoryStatusEx`, `GetDiskFreeSpaceExW`, and the registry reads
// below in particular — is the single most likely place in this PR to
// need a real-compiler fixup, the same category of finding every prior
// Rust task's first real gate run in this project has hit (T-003's
// `clippy::type_complexity`, T-004's `cargo fmt` line-wrap). Every
// function here still holds its own invariant regardless: a failure
// degrades to `0` or `"unknown"`, never a panic, so a signature mismatch
// caught by the real compiler is the worst case, not a runtime crash
// masked by one.

fn system_ram_bytes() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `status` is a validly-sized, exclusively-owned buffer for
    // the duration of this call, and `dwLength` is set as the API requires.
    match unsafe { GlobalMemoryStatusEx(&mut status) } {
        Ok(()) => status.ullTotalPhys,
        Err(_) => 0,
    }
}

fn app_data_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| std::path::PathBuf::from(base).join("LlamaManager"))
}

fn free_disk_bytes() -> u64 {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let Some(dir) = app_data_dir() else {
        return 0;
    };
    let path = HSTRING::from(dir.as_os_str());
    let mut free_to_caller: u64 = 0;
    // SAFETY: `free_to_caller` is a valid `u64` local for the duration of
    // the call; the other two out-parameters are not requested (`None`).
    match unsafe { GetDiskFreeSpaceExW(&path, Some(&mut free_to_caller), None, None) } {
        Ok(()) => free_to_caller,
        Err(_) => 0,
    }
}

fn os_build_string() -> String {
    match read_current_version() {
        Some((product_name, Some(build), Some(ubr))) => format!("{product_name} {build}.{ubr}"),
        Some((product_name, Some(build), None)) => format!("{product_name} {build}"),
        Some((product_name, None, _)) => product_name,
        None => "unknown".to_string(),
    }
}

/// Reads `ProductName`, `CurrentBuildNumber` and `UBR` from
/// `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`. Deliberately not
/// `GetVersionEx` (deprecated, and lies about the OS version under an
/// application manifest that doesn't declare Windows 11 support) and not
/// `RtlGetVersion` (`ntdll`, outside the `windows` crate's ordinary Win32
/// surface) — this is the same registry path most real Windows-version
/// tooling reads.
fn read_current_version() -> Option<(String, Option<String>, Option<u32>)> {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ};

    let mut hkey = HKEY::default();
    // SAFETY: `hkey` is only ever read through the registry API below and
    // closed on every path before this function returns.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            0,
            KEY_READ,
            &mut hkey,
        )
    };
    if opened.is_err() {
        return None;
    }

    let product_name = read_registry_string(hkey, w!("ProductName")).unwrap_or_else(|| "Windows".to_string());
    let build = read_registry_string(hkey, w!("CurrentBuildNumber"));
    let ubr = read_registry_u32(hkey, w!("UBR"));

    // SAFETY: `hkey` was successfully opened above and is not used again
    // after this call.
    let _ = unsafe { RegCloseKey(hkey) };

    Some((product_name, build, ubr))
}

fn read_registry_string(hkey: windows::Win32::System::Registry::HKEY, name: windows::core::PCWSTR) -> Option<String> {
    use windows::Win32::System::Registry::RegQueryValueExW;

    let mut byte_len: u32 = 0;
    // First call: discover the buffer size. SAFETY: every out-pointer is
    // either `None` or a valid local for the duration of the call.
    let sized = unsafe { RegQueryValueExW(hkey, name, None, None, None, Some(&mut byte_len)) };
    if sized.is_err() || byte_len == 0 {
        return None;
    }

    let mut buffer: Vec<u16> = vec![0u16; (byte_len as usize) / 2 + 1];
    let mut actual_len = byte_len;
    // SAFETY: `buffer` is sized from the length the first call reported,
    // with one extra `u16` of slack in case the stored value has no NUL
    // terminator.
    let read = unsafe {
        RegQueryValueExW(
            hkey,
            name,
            None,
            None,
            Some(buffer.as_mut_ptr() as *mut u8),
            Some(&mut actual_len),
        )
    };
    if read.is_err() {
        return None;
    }

    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn read_registry_u32(hkey: windows::Win32::System::Registry::HKEY, name: windows::core::PCWSTR) -> Option<u32> {
    use windows::Win32::System::Registry::RegQueryValueExW;

    let mut value: u32 = 0;
    let mut byte_len: u32 = std::mem::size_of::<u32>() as u32;
    // SAFETY: `value` is a valid 4-byte buffer matching `byte_len`.
    let read = unsafe {
        RegQueryValueExW(
            hkey,
            name,
            None,
            None,
            Some(&mut value as *mut u32 as *mut u8),
            Some(&mut byte_len),
        )
    };
    if read.is_err() {
        None
    } else {
        Some(value)
    }
}

// ─── Real (non-mock) NVML backing (`nvml-wrapper`, `PLAN.md` §3) ──────
//
// The only non-test implementation of `NvmlProvider` this task ships.
// `Nvml::init()` itself can fail — no NVIDIA driver, no NVML library on
// `PATH` — and that failure degrades the same way a later per-call
// failure does: a typed `AppError`, never unwrapped, never panicking.
// Held behind a `OnceLock` so a machine without an NVIDIA driver pays the
// `Nvml::init()` cost, and its failure, once per process rather than once
// per probe.
//
// **Unverified against a real compile**, same caveat as the system-
// information block above: `nvml-wrapper`'s exact method names and return
// shapes are this session's best recollection of the crate, not something
// checked against its docs (no network access to docs.rs, no toolchain to
// let `cargo build` catch a mismatch). Every method here still degrades to
// a typed error on any failure, so the worst case is a compile error for
// the owner's real toolchain to fix, not a masked panic.

pub struct RealNvmlProvider;

impl RealNvmlProvider {
    pub fn new() -> Self {
        Self
    }

    fn nvml() -> Result<&'static nvml_wrapper::Nvml, AppError> {
        static NVML: OnceLock<Result<nvml_wrapper::Nvml, String>> = OnceLock::new();
        match NVML.get_or_init(|| nvml_wrapper::Nvml::init().map_err(|err| err.to_string())) {
            Ok(nvml) => Ok(nvml),
            Err(message) => Err(AppError::Internal {
                message: format!("NVML init failed: {message}"),
            }),
        }
    }
}

impl Default for RealNvmlProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn nvml_err(err: nvml_wrapper::error::NvmlError) -> AppError {
    AppError::Internal {
        message: format!("NVML error: {err}"),
    }
}

impl NvmlProvider for RealNvmlProvider {
    fn device_count(&self) -> Result<u32, AppError> {
        Self::nvml()?.device_count().map_err(nvml_err)
    }

    fn device_info(&self, index: u32) -> Result<GpuInfo, AppError> {
        let nvml = Self::nvml()?;
        let device = nvml.device_by_index(index).map_err(nvml_err)?;
        let name = device.name().map_err(nvml_err)?;
        let cc = device.cuda_compute_capability().map_err(nvml_err)?;
        let mem = device.memory_info().map_err(nvml_err)?;
        let driver = nvml.sys_driver_version().map_err(nvml_err)?;
        let cuda_version = nvml
            .sys_cuda_driver_version()
            .ok()
            .map(|v| format!("{}.{}", v / 1000, (v % 1000) / 10));

        Ok(GpuInfo {
            index,
            name,
            compute_capability: (cc.major as u32, cc.minor as u32),
            vram_total_bytes: mem.total,
            vram_free_bytes: mem.free,
            driver_version: driver,
            cuda_version,
        })
    }

    fn device_telemetry(&self, index: u32) -> Result<GpuTelemetry, AppError> {
        // T-060/T-061's job to call this in earnest; implemented honestly
        // now rather than left as a stub, so the trait is not partially real.
        let device = Self::nvml()?.device_by_index(index).map_err(nvml_err)?;
        let mem = device.memory_info().map_err(nvml_err)?;
        let util = device.utilization_rates().map_err(nvml_err)?;
        let temperature_c = device
            .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
            .ok();

        Ok(GpuTelemetry {
            index,
            vram_used_bytes: mem.used,
            vram_total_bytes: mem.total,
            utilization_percent: util.gpu,
            temperature_c,
        })
    }

    fn driver_version(&self) -> Result<String, AppError> {
        Self::nvml()?.sys_driver_version().map_err(nvml_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canned `NvmlProvider`: `device_count`, each `device_info` call and
    /// `driver_version` all return a fixed `Result` set at construction
    /// time. Nothing here reaches real hardware or a real NVML library —
    /// that is the whole point of the trait (`docs/CONTRACTS.md` §6).
    struct MockNvmlProvider {
        count: Result<u32, AppError>,
        infos: Vec<Result<GpuInfo, AppError>>,
        driver: Result<String, AppError>,
    }

    impl NvmlProvider for MockNvmlProvider {
        fn device_count(&self) -> Result<u32, AppError> {
            self.count.clone()
        }

        fn device_info(&self, index: u32) -> Result<GpuInfo, AppError> {
            self.infos
                .get(index as usize)
                .cloned()
                .unwrap_or_else(|| {
                    Err(AppError::NotFound {
                        what: format!("gpu index {index}"),
                    })
                })
        }

        fn device_telemetry(&self, _index: u32) -> Result<GpuTelemetry, AppError> {
            Err(AppError::Internal {
                message: "telemetry not modelled by this fixture".to_string(),
            })
        }

        fn driver_version(&self) -> Result<String, AppError> {
            self.driver.clone()
        }
    }

    fn nvml_error(message: &str) -> AppError {
        AppError::Internal {
            message: message.to_string(),
        }
    }

    /// Blackwell — `GpuInfo`'s own doc comment: "Blackwell = (12, 0) or
    /// (10, x)". Driver comfortably above both floors this file defines.
    fn blackwell_gpu() -> GpuInfo {
        GpuInfo {
            index: 0,
            name: "NVIDIA GeForce RTX 5090".to_string(),
            compute_capability: (12, 0),
            vram_total_bytes: 32 * 1024 * 1024 * 1024,
            vram_free_bytes: 30 * 1024 * 1024 * 1024,
            driver_version: "581.29".to_string(),
            cuda_version: Some("13.0".to_string()),
        }
    }

    /// Ada Lovelace — compute capability (8, 9), verified against NVIDIA's
    /// own `developer.nvidia.com/cuda/gpus` and the Ada Compatibility
    /// Guide rather than assumed, per this task's own instruction to check
    /// it in the same spirit as `AGENTS.md` §1's flag caution. Driver
    /// current enough to pass both floors, same as the Blackwell fixture —
    /// this profile exists to prove a non-Blackwell GPU with a healthy
    /// driver reports all-Pass too, not to test the driver floor (the
    /// driver-too-old profile below does that).
    fn ada_gpu() -> GpuInfo {
        GpuInfo {
            index: 0,
            name: "NVIDIA GeForce RTX 4090".to_string(),
            compute_capability: (8, 9),
            vram_total_bytes: 24 * 1024 * 1024 * 1024,
            vram_free_bytes: 22 * 1024 * 1024 * 1024,
            driver_version: "581.29".to_string(),
            cuda_version: Some("13.0".to_string()),
        }
    }

    /// Every `HealthCheck` in `report.checks` whose status is Warn or Fail
    /// carries non-empty remediation — the acceptance criterion asked for
    /// directly, checked once here and called from every test below rather
    /// than repeated per test.
    fn assert_remediation_invariant(report: &EnvironmentReport) {
        for check in &report.checks {
            if !matches!(check.status, CheckStatus::Pass) {
                assert!(
                    check
                        .remediation
                        .as_ref()
                        .is_some_and(|r| !r.is_empty()),
                    "check `{}` is {:?} but has no remediation",
                    check.id,
                    check.status
                );
            }
        }
    }

    fn find_check<'a>(report: &'a EnvironmentReport, id: &str) -> &'a HealthCheck {
        report
            .checks
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no check with id `{id}` in report"))
    }

    fn assert_check_status(report: &EnvironmentReport, id: &str, expected: CheckStatus) {
        let check = find_check(report, id);
        assert_eq!(
            check.status, expected,
            "check `{id}` expected {expected:?}, got {:?}: {}",
            check.status, check.message
        );
    }

    fn free_ephemeral_port() -> u16 {
        // Bind to port 0 to let the OS choose a free one, then release it
        // immediately — there is a race in principle (something else could
        // grab it before the test's own bind), but it is the same race
        // every "find a free port" strategy has without a privileged
        // reservation API, and is the standard approach.
        let listener = TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .expect("binding to an OS-chosen port must succeed on any CI runner");
        listener
            .local_addr()
            .expect("a bound listener always has a local address")
            .port()
    }

    #[test]
    fn blackwell_profile_all_checks_pass() {
        let provider = MockNvmlProvider {
            count: Ok(1),
            infos: vec![Ok(blackwell_gpu())],
            driver: Ok("581.29".to_string()),
        };
        let report = probe(&provider, free_ephemeral_port());

        assert_eq!(report.gpus.len(), 1);
        assert_check_status(&report, "gpu_present", CheckStatus::Pass);
        assert_check_status(&report, "driver_ok", CheckStatus::Pass);
        assert_check_status(&report, "cuda13_ok", CheckStatus::Pass);
        assert_remediation_invariant(&report);
    }

    #[test]
    fn ada_profile_current_driver_all_pass() {
        let provider = MockNvmlProvider {
            count: Ok(1),
            infos: vec![Ok(ada_gpu())],
            driver: Ok("581.29".to_string()),
        };
        let report = probe(&provider, free_ephemeral_port());

        assert_eq!(report.gpus.len(), 1);
        assert_check_status(&report, "gpu_present", CheckStatus::Pass);
        assert_check_status(&report, "driver_ok", CheckStatus::Pass);
        assert_check_status(&report, "cuda13_ok", CheckStatus::Pass);
        assert_remediation_invariant(&report);
    }

    #[test]
    fn driver_too_old_fails_driver_and_cuda13_checks_but_not_gpu_present() {
        let mut gpu = ada_gpu();
        gpu.driver_version = "470.10".to_string();
        let provider = MockNvmlProvider {
            count: Ok(1),
            infos: vec![Ok(gpu)],
            driver: Ok("470.10".to_string()),
        };
        let report = probe(&provider, free_ephemeral_port());

        assert_eq!(report.gpus.len(), 1);
        assert_check_status(&report, "gpu_present", CheckStatus::Pass);
        assert_check_status(&report, "driver_ok", CheckStatus::Fail);
        assert_check_status(&report, "cuda13_ok", CheckStatus::Fail);
        assert_remediation_invariant(&report);
    }

    #[test]
    fn driver_between_floors_passes_general_but_fails_cuda13() {
        // Exercises the distinction between the two floors directly: a
        // driver at 550 clears `MIN_DRIVER_MAJOR_GENERAL` (525) but not
        // `MIN_DRIVER_MAJOR_CUDA13` (580).
        let mut gpu = ada_gpu();
        gpu.driver_version = "550.54.14".to_string();
        let provider = MockNvmlProvider {
            count: Ok(1),
            infos: vec![Ok(gpu)],
            driver: Ok("550.54.14".to_string()),
        };
        let report = probe(&provider, free_ephemeral_port());

        assert_check_status(&report, "gpu_present", CheckStatus::Pass);
        assert_check_status(&report, "driver_ok", CheckStatus::Pass);
        assert_check_status(&report, "cuda13_ok", CheckStatus::Fail);
        assert_remediation_invariant(&report);
    }

    #[test]
    fn no_gpu_profile_fails_gpu_present_with_remediation_and_does_not_panic() {
        let provider = MockNvmlProvider {
            count: Ok(0),
            infos: vec![],
            driver: Err(nvml_error("no driver")),
        };
        let report = probe(&provider, free_ephemeral_port());

        assert!(report.gpus.is_empty());
        assert_check_status(&report, "gpu_present", CheckStatus::Fail);
        assert!(
            find_check(&report, "gpu_present")
                .remediation
                .as_ref()
                .is_some_and(|r| !r.is_empty())
        );
        assert_remediation_invariant(&report);
    }

    #[test]
    fn nvml_error_on_device_count_degrades_to_no_gpu_without_panicking() {
        let provider = MockNvmlProvider {
            count: Err(nvml_error("nvmlInit failed")),
            infos: vec![],
            driver: Err(nvml_error("nvmlInit failed")),
        };
        let report = probe(&provider, free_ephemeral_port());

        assert!(report.gpus.is_empty());
        assert_check_status(&report, "gpu_present", CheckStatus::Fail);
        assert_remediation_invariant(&report);
    }

    #[test]
    fn nvml_error_mid_call_on_one_device_is_handled_not_propagated() {
        let provider = MockNvmlProvider {
            count: Ok(2),
            infos: vec![
                Ok(blackwell_gpu()),
                Err(nvml_error("nvmlDeviceGetHandleByIndex failed")),
            ],
            driver: Ok("581.29".to_string()),
        };
        let report = probe(&provider, free_ephemeral_port());

        // One of two devices failed mid-call; the probe reports the one
        // that succeeded rather than aborting the whole report.
        assert_eq!(report.gpus.len(), 1);
        assert_check_status(&report, "gpu_present", CheckStatus::Pass);
        assert_remediation_invariant(&report);
    }

    #[test]
    fn nvml_error_on_every_device_info_call_degrades_to_no_gpu() {
        let provider = MockNvmlProvider {
            count: Ok(1),
            infos: vec![Err(nvml_error("nvmlDeviceGetHandleByIndex failed"))],
            driver: Ok("581.29".to_string()),
        };
        let report = probe(&provider, free_ephemeral_port());

        assert!(report.gpus.is_empty());
        assert_check_status(&report, "gpu_present", CheckStatus::Fail);
        assert_remediation_invariant(&report);
    }

    #[test]
    fn endpoint_bindable_binds_then_releases_the_port() {
        let provider = MockNvmlProvider {
            count: Ok(0),
            infos: vec![],
            driver: Err(nvml_error("no driver")),
        };
        let port = free_ephemeral_port();

        let report = probe(&provider, port);
        assert_check_status(&report, "endpoint_bindable", CheckStatus::Pass);

        // Proves the port was released, not just reported free: bind it
        // again ourselves, which only succeeds if nothing still holds it.
        let relisten = TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), port));
        assert!(relisten.is_ok(), "port {port} was not released after the check");
    }

    #[test]
    fn endpoint_bindable_reports_fail_when_port_is_held() {
        let holder = TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .expect("binding to an OS-chosen port must succeed on any CI runner");
        let held_port = holder
            .local_addr()
            .expect("a bound listener always has a local address")
            .port();

        let provider = MockNvmlProvider {
            count: Ok(0),
            infos: vec![],
            driver: Err(nvml_error("no driver")),
        };
        let report = probe(&provider, held_port);

        assert_check_status(&report, "endpoint_bindable", CheckStatus::Fail);
        assert_remediation_invariant(&report);
        drop(holder); // keep the listener alive for the whole check above
    }

    // ─── disk_space: pure decision function, no real filesystem I/O ────

    #[test]
    fn disk_status_fails_below_the_fail_floor() {
        let check = disk_status(DISK_FAIL_BELOW_BYTES - 1);
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.remediation.as_ref().is_some_and(|r| !r.is_empty()));
    }

    #[test]
    fn disk_status_warns_between_the_two_floors() {
        let check = disk_status(DISK_FAIL_BELOW_BYTES + 1);
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.remediation.as_ref().is_some_and(|r| !r.is_empty()));
    }

    #[test]
    fn disk_status_passes_at_or_above_the_warn_floor() {
        let check = disk_status(DISK_WARN_BELOW_BYTES);
        assert_eq!(check.status, CheckStatus::Pass);
        assert!(check.remediation.is_none());
    }

    #[test]
    fn disk_status_id_is_always_disk_space() {
        for bytes in [0, DISK_FAIL_BELOW_BYTES, DISK_WARN_BELOW_BYTES, u64::MAX] {
            assert_eq!(disk_status(bytes).id, "disk_space");
        }
    }

    // ─── driver_major: pure string parsing ──────────────────────────────

    #[test]
    fn driver_major_parses_the_leading_component() {
        assert_eq!(driver_major("580.65.06"), Some(580));
        assert_eq!(driver_major("470.10"), Some(470));
        assert_eq!(driver_major("581"), Some(581));
    }

    #[test]
    fn driver_major_never_panics_on_garbage() {
        assert_eq!(driver_major(""), None);
        assert_eq!(driver_major("not-a-version"), None);
        assert_eq!(driver_major("."), None);
        assert_eq!(driver_major("580abc.1"), None);
    }

    #[test]
    fn report_always_carries_exactly_the_five_known_check_ids() {
        // `HealthCheck.id`'s own doc comment enumerates exactly five ids.
        // T-011 renders a fixed set of traffic lights against them, so a
        // profile that silently drops or adds one would break that screen
        // without any test here catching it.
        let provider = MockNvmlProvider {
            count: Ok(0),
            infos: vec![],
            driver: Err(nvml_error("no driver")),
        };
        let report = probe(&provider, free_ephemeral_port());
        let mut ids: Vec<&str> = report.checks.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec!["cuda13_ok", "disk_space", "driver_ok", "endpoint_bindable", "gpu_present"]
        );
    }
}
