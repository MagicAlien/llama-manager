//! T-021 — Backend selection.
//!
//! A pure function from an `EnvironmentReport` (T-010) plus the available
//! releases (T-020) to a chosen `AvailableRelease`. No I/O: the function
//! takes already-fetched data and returns a decision.
//!
//! Rules, per `PLAN.md` §2.2:
//!
//! - **Blackwell** (any GPU with compute capability major >= 10) takes a
//!   CUDA build of major >= 13, or fails. "Blackwell present, no suitable
//!   CUDA asset" is a hard failure with a message naming the
//!   silent-CPU-fallback risk — not a downgrade, not a warning-and-proceed.
//!   The failure being avoided is a silent fallback to CPU inside the
//!   binary, which no warning can recover from once inference has started.
//! - **Non-Blackwell NVIDIA** prefers the highest available CUDA major and
//!   may fall back to a lower major the driver can run.
//! - **No NVIDIA GPU**, or a driver too old for the selected CUDA major,
//!   yields Vulkan; CPU is last.
//! - **Vulkan and CPU** selections carry an explicit warning that
//!   performance will be poor.
//!
//! **Driver floors** (`PROGRESS.md` F-004, from NVIDIA's published "Driver
//! Range for Minor Version Compatibility" table): CUDA 13 needs driver >=
//! 580; CUDA 12 needs >= 525; CUDA 11 needs >= 450. A CUDA major the code
//! has no floor for (>= 14) is treated conservatively as needing the
//! highest known floor (580) — a necessary but not sufficient condition, and
//! the best available approximation without a published floor. A major below
//! 11 has no published floor and is treated as unconfirmable (never
//! selected).
//!
//! **Readings recorded for the owner** (see the PR description):
//!
//! - "Blackwell with a CUDA major above 13 available" is listed as a
//!   required profile without a stated outcome. This implementation selects
//!   the highest available CUDA major >= 13 (a higher major is
//!   forward-compatible) — the defensible reading the task names.
//! - The hard failure is scoped to the *no-asset* case ("Blackwell present,
//!   no suitable CUDA asset"), per `PLAN.md` §2.2's own wording. A Blackwell
//!   GPU with a suitable asset but a driver too old for it falls back to
//!   Vulkan (the general driver rule), not a hard failure — the driver issue
//!   is fixable (update the driver) and Vulkan works on Blackwell in the
//!   meantime.
//!
//! **Gate trap** (`PROGRESS.md` T-020 note): this crate is binary-only, so
//! an uncalled `pub` item fails `cargo clippy -- -D warnings` with
//! `dead_code`. T-022 (the runtime installer) is the consumer and has not
//! landed yet, so nothing in this branch calls `select_backend`. The task
//! scope excludes `ipc/` wiring and `main.rs` changes, so the resolution is
//! the module-level `#![allow(dead_code)]` below (T-010's precedent for
//! `probe` before T-011 wired `ipc::probe_environment`), to be removed when
//! T-022 wires in the caller. Recorded in the PR description.

// T-022 is the consumer of `select_backend` and has not landed yet, so this
// module's public surface has no caller in this branch and would otherwise
// fail `cargo clippy -- -D warnings` with `dead_code` (binary-only crate —
// see the module doc's gate-trap note). Suppressed at the module level, the
// same way T-010 did for `probe`, and removed when T-022 wires the caller.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::core::types::{AppError, AvailableRelease, Backend, EnvironmentReport};

/// Blackwell threshold: compute capability major >= 10 (RTX 50xx / RTX PRO,
/// CC (12, 0) or (10, x)). `PLAN.md` §2.2.
const BLACKWELL_CC_MAJOR: u32 = 10;

/// Minimum CUDA major a Blackwell build must be. `PLAN.md` §2.2: "Blackwell
/// takes the CUDA 13 build or nothing."
const MIN_CUDA_MAJOR_BLACKWELL: u8 = 13;

/// The poor-performance warning carried by every Vulkan and CPU selection.
/// `PLAN.md` §2.2: "Vulkan and CPU builds remain diagnostic fallbacks only,
/// surfaced with an explicit warning that performance will be poor."
const POOR_PERFORMANCE_WARNING: &str = "This build will run without full GPU \
                                        acceleration; expect much lower performance.";

/// The result of [`select_backend`]: the chosen release plus an optional
/// warning. The warning is `Some` for Vulkan and CPU selections (performance
/// will be poor) and `None` for CUDA selections.
///
/// `#[ts(export)]` so the frontend (T-024, the version-management UI) can
/// display the selection and its warning; `AvailableRelease` is already
/// exported, and this is a thin wrapper around it.
#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct BackendSelection {
    pub release: AvailableRelease,
    pub warning: Option<String>,
}

/// Select a backend asset from the available releases for the given
/// environment. Pure: no I/O, takes already-fetched data.
///
/// See the module doc for the rules. Returns `Err(AppError::NotFound)` for
/// the hard-failure cases: an empty release list, a Blackwell GPU with no
/// CUDA >= 13 asset (the silent-CPU-fallback risk), and a fallback that
/// finds neither a Vulkan nor a CPU release. Never panics.
pub fn select_backend(
    report: &EnvironmentReport,
    releases: &[AvailableRelease],
) -> Result<BackendSelection, AppError> {
    // An empty release list is a typed hard failure, never a panic: there is
    // nothing to select from.
    if releases.is_empty() {
        return Err(AppError::NotFound {
            what: "no releases are available to select from".to_string(),
        });
    }

    let blackwell = report
        .gpus
        .iter()
        .any(|gpu| gpu.compute_capability.0 >= BLACKWELL_CC_MAJOR);
    let has_nvidia = !report.gpus.is_empty();
    // `gpus` holds only NVIDIA GPUs (T-010 probes via NVML), so a non-empty
    // `gpus` is exactly "has an NVIDIA GPU". The driver is system-wide; take
    // the first GPU that reports a version (T-010's `driver_checks` uses the
    // same first-GPU fallback).
    let driver_major = report
        .gpus
        .iter()
        .find(|gpu| !gpu.driver_version.is_empty())
        .and_then(|gpu| driver_major(&gpu.driver_version));

    if blackwell {
        select_for_blackwell(releases, driver_major)
    } else if has_nvidia {
        select_for_non_blackwell(releases, driver_major)
    } else {
        select_fallback(releases)
    }
}

/// Blackwell: select the highest CUDA major >= 13 the driver can run. Hard
/// failure if no CUDA >= 13 asset exists (the silent-CPU-fallback risk). If
/// the driver is too old (or unknown) for the selected major, fall back to
/// Vulkan/CPU (the general driver rule).
fn select_for_blackwell(
    releases: &[AvailableRelease],
    driver_major: Option<u32>,
) -> Result<BackendSelection, AppError> {
    // The highest available CUDA major >= 13. T-020 returns at most one
    // release per CUDA major (newest per backend), so this is unambiguous.
    let best = releases
        .iter()
        .filter(|release| cuda_major(&release.backend) >= MIN_CUDA_MAJOR_BLACKWELL)
        .max_by_key(|release| cuda_major(&release.backend));

    let Some(release) = best else {
        // Blackwell present, no suitable CUDA asset. Hard failure: the
        // silent-CPU-fallback risk. Not a downgrade, not a warning.
        return Err(AppError::NotFound {
            what: "no CUDA 13+ build is available for this Blackwell GPU; \
                    selecting a lower CUDA build would silently fall back to \
                    CPU inside the binary, which no warning can recover from \
                    once inference has started"
                .to_string(),
        });
    };

    let selected_major = cuda_major(&release.backend);
    if driver_can_run(driver_major, selected_major) {
        Ok(BackendSelection {
            release: release.clone(),
            warning: None,
        })
    } else {
        // The driver is too old (or unknown) for the selected CUDA major.
        // The general rule (`PLAN.md` §2.2) yields Vulkan; CPU is last.
        select_fallback(releases)
    }
}

/// Non-Blackwell NVIDIA: prefer the highest available CUDA major the driver
/// can run, falling back to a lower major, then Vulkan, then CPU.
fn select_for_non_blackwell(
    releases: &[AvailableRelease],
    driver_major: Option<u32>,
) -> Result<BackendSelection, AppError> {
    // The CUDA releases, highest major first.
    let mut cuda: Vec<&AvailableRelease> = releases
        .iter()
        .filter(|release| is_cuda(&release.backend))
        .collect();
    cuda.sort_by_key(|release| std::cmp::Reverse(cuda_major(&release.backend)));

    // Prefer the highest major the driver can run.
    for release in cuda.iter().copied() {
        let major = cuda_major(&release.backend);
        if driver_can_run(driver_major, major) {
            return Ok(BackendSelection {
                release: release.clone(),
                warning: None,
            });
        }
    }

    // No CUDA build the driver can run. Vulkan, then CPU.
    select_fallback(releases)
}

/// Vulkan, then CPU. Both carry the poor-performance warning.
fn select_fallback(releases: &[AvailableRelease]) -> Result<BackendSelection, AppError> {
    let warning = Some(POOR_PERFORMANCE_WARNING.to_string());
    if let Some(release) = releases
        .iter()
        .find(|release| release.backend == Backend::Vulkan)
    {
        return Ok(BackendSelection {
            release: release.clone(),
            warning,
        });
    }
    if let Some(release) = releases
        .iter()
        .find(|release| release.backend == Backend::Cpu)
    {
        return Ok(BackendSelection {
            release: release.clone(),
            warning,
        });
    }
    // Neither Vulkan nor CPU is available.
    Err(AppError::NotFound {
        what: "no Vulkan or CPU build is available to fall back to".to_string(),
    })
}

/// The CUDA major of a backend, or 0 for a non-CUDA backend.
fn cuda_major(backend: &Backend) -> u8 {
    match backend {
        Backend::Cuda { major } => *major,
        _ => 0,
    }
}

/// Whether the backend is a CUDA build.
fn is_cuda(backend: &Backend) -> bool {
    matches!(backend, Backend::Cuda { .. })
}

/// Parse the leading dot-separated major component of an NVML driver version
/// string (`"580.65.06"` -> `Some(580)`). Never panics: a string that does
/// not start with an integer yields `None`. (Same as T-010's `driver_major`.)
fn driver_major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse::<u32>().ok()
}

/// The minimum NVIDIA driver branch (major) required to run a CUDA build of
/// the given major. `PROGRESS.md` F-004, from NVIDIA's published "Driver
/// Range for Minor Version Compatibility" table: 13.x needs >= 580; 12.x
/// needs >= 525; 11.x needs >= 450. A major the code has no floor for (>=
/// 14) is treated conservatively as the highest known floor (580). A major
/// below 11 has no published floor and is unconfirmable (`None`).
fn driver_floor_for_cuda(major: u8) -> Option<u32> {
    match major {
        13.. => Some(580),
        12 => Some(525),
        11 => Some(450),
        _ => None,
    }
}

/// Whether a driver of the given major can run a CUDA build of the given
/// major. `false` when either the driver major or the floor is unknown — we
/// never select a CUDA build we cannot confirm the driver can run (the
/// property test's first clause).
fn driver_can_run(driver_major: Option<u32>, cuda_major: u8) -> bool {
    match (driver_major, driver_floor_for_cuda(cuda_major)) {
        (Some(driver), Some(floor)) => driver >= floor,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::GpuInfo;
    use chrono::{TimeZone, Utc};
    use proptest::prelude::*;

    /// A realistic GPU fixture. `cc` is the compute capability; `driver` is
    /// a real NVIDIA driver version string ("" for the unknown-driver case).
    fn gpu(name: &str, cc: (u32, u32), driver: &str) -> GpuInfo {
        GpuInfo {
            index: 0,
            name: name.to_string(),
            compute_capability: cc,
            vram_total_bytes: 24 * 1024 * 1024 * 1024,
            vram_free_bytes: 20 * 1024 * 1024 * 1024,
            driver_version: driver.to_string(),
            cuda_version: None,
        }
    }

    fn report(gpus: Vec<GpuInfo>) -> EnvironmentReport {
        EnvironmentReport {
            gpus,
            system_ram_bytes: 64 * 1024 * 1024 * 1024,
            free_disk_bytes: 500 * 1024 * 1024 * 1024,
            os_build: "Windows 11 22631".to_string(),
            checks: Vec::new(),
        }
    }

    fn release(backend: Backend, build_tag: &str) -> AvailableRelease {
        AvailableRelease {
            build_tag: build_tag.to_string(),
            backend,
            asset_name: format!("llama-{build_tag}-bin-win-x64.zip"),
            asset_url: format!("https://example.invalid/{build_tag}.zip"),
            sha256: None,
            size_bytes: 1024,
            published_at: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
            release_notes_url: format!(
                "https://github.com/ggml-org/llama.cpp/releases/tag/{build_tag}"
            ),
        }
    }

    /// The expected outcome of a profile.
    #[derive(Debug)]
    enum Expected {
        /// A CUDA build of this major, no warning.
        Cuda(u8),
        /// A Vulkan build, with the poor-performance warning.
        Vulkan,
        /// A CPU build, with the poor-performance warning.
        Cpu,
        /// A hard failure (`AppError::NotFound`).
        NotFound,
    }

    /// Table-driven over realistic hardware profiles. GPU names, compute
    /// capabilities and driver versions are all real: RTX 5090 is Blackwell
    /// (CC 12.0), RTX 4090 is Ada Lovelace (CC 8.9). Driver versions are real
    /// published NVIDIA Windows releases: 580.88 (R580, the CUDA 13 floor and
    /// its Windows datacenter counterpart of 580.65.06), 572.16 (CUDA 12,
    /// below the 13 floor), 551.86 (between the 12 and 13 floors), 512.15
    /// (R510, below the 12 floor).
    #[test]
    fn select_backend_table_driven() {
        let profiles: &[(&str, Vec<GpuInfo>, Vec<Backend>, Expected)] = &[
            // 1. Blackwell + CUDA 13 available, driver OK -> CUDA 13.
            (
                "Blackwell + CUDA 13, driver OK",
                vec![gpu("NVIDIA GeForce RTX 5090", (12, 0), "580.88")],
                vec![
                    Backend::Cuda { major: 13 },
                    Backend::Cuda { major: 12 },
                    Backend::Vulkan,
                    Backend::Cpu,
                ],
                Expected::Cuda(13),
            ),
            // 2. Blackwell + only CUDA 12 available -> hard failure (the
            //    silent-CPU-fallback risk).
            (
                "Blackwell + only CUDA 12",
                vec![gpu("NVIDIA GeForce RTX 5090", (12, 0), "580.88")],
                vec![Backend::Cuda { major: 12 }, Backend::Vulkan, Backend::Cpu],
                Expected::NotFound,
            ),
            // 3. Blackwell + a CUDA major above 13 available -> select the
            //    highest (CUDA 14), per the recorded reading.
            (
                "Blackwell + CUDA 14, driver OK",
                vec![gpu("NVIDIA GeForce RTX 5090", (12, 0), "580.88")],
                vec![
                    Backend::Cuda { major: 14 },
                    Backend::Cuda { major: 13 },
                    Backend::Vulkan,
                    Backend::Cpu,
                ],
                Expected::Cuda(14),
            ),
            // 4. Blackwell + CUDA 13 available but the driver is too old ->
            //    Vulkan (the general driver rule), with a warning.
            (
                "Blackwell + CUDA 13, old driver",
                vec![gpu("NVIDIA GeForce RTX 5090", (12, 0), "572.16")],
                vec![Backend::Cuda { major: 13 }, Backend::Vulkan, Backend::Cpu],
                Expected::Vulkan,
            ),
            // 5. Blackwell + CUDA 13 available but the driver is unknown ->
            //    Vulkan (cannot confirm the driver can run it), with a
            //    warning.
            (
                "Blackwell + CUDA 13, unknown driver",
                vec![gpu("NVIDIA GeForce RTX 5090", (12, 0), "")],
                vec![Backend::Cuda { major: 13 }, Backend::Vulkan, Backend::Cpu],
                Expected::Vulkan,
            ),
            // 6. Non-Blackwell (Ada) + CUDA 13 available, driver OK -> CUDA
            //    13 (the highest).
            (
                "Ada + CUDA 13, driver OK",
                vec![gpu("NVIDIA GeForce RTX 4090", (8, 9), "580.88")],
                vec![
                    Backend::Cuda { major: 13 },
                    Backend::Cuda { major: 12 },
                    Backend::Vulkan,
                    Backend::Cpu,
                ],
                Expected::Cuda(13),
            ),
            // 7. Non-Blackwell (Ada) + an old driver (below the CUDA 12
            //    floor) -> Vulkan, with a warning.
            (
                "Ada + old driver",
                vec![gpu("NVIDIA GeForce RTX 4090", (8, 9), "512.15")],
                vec![
                    Backend::Cuda { major: 13 },
                    Backend::Cuda { major: 12 },
                    Backend::Vulkan,
                    Backend::Cpu,
                ],
                Expected::Vulkan,
            ),
            // 8. Non-Blackwell (Ada) + a driver that can run CUDA 12 but not
            //    CUDA 13 -> fall back to CUDA 12, no warning.
            (
                "Ada + driver OK for CUDA 12 only",
                vec![gpu("NVIDIA GeForce RTX 4090", (8, 9), "551.86")],
                vec![
                    Backend::Cuda { major: 13 },
                    Backend::Cuda { major: 12 },
                    Backend::Vulkan,
                    Backend::Cpu,
                ],
                Expected::Cuda(12),
            ),
            // 9. No GPU -> Vulkan, with a warning.
            (
                "No GPU",
                Vec::new(),
                vec![Backend::Vulkan, Backend::Cpu],
                Expected::Vulkan,
            ),
            // 10. No GPU + only a CPU release available -> CPU, with a
            //     warning.
            (
                "No GPU, only CPU",
                Vec::new(),
                vec![Backend::Cpu],
                Expected::Cpu,
            ),
            // 11. An empty release list -> a typed hard failure, never a
            //     panic.
            (
                "Empty release list",
                vec![gpu("NVIDIA GeForce RTX 5090", (12, 0), "580.88")],
                Vec::new(),
                Expected::NotFound,
            ),
            // 12. Non-Blackwell (Ada) + no CUDA release, only Vulkan ->
            //     Vulkan, with a warning.
            (
                "Ada + no CUDA, only Vulkan",
                vec![gpu("NVIDIA GeForce RTX 4090", (8, 9), "580.88")],
                vec![Backend::Vulkan, Backend::Cpu],
                Expected::Vulkan,
            ),
            // 13. No GPU + both Vulkan and CPU available -> Vulkan (preferred over
            //     CPU), with a warning.
            (
                "No GPU, Vulkan and CPU",
                Vec::new(),
                vec![Backend::Cpu, Backend::Vulkan],
                Expected::Vulkan,
            ),
        ];

        assert!(
            profiles.len() >= 10,
            "T-021 acceptance: at least 10 hardware profiles"
        );

        for (name, gpus, backends, expected) in profiles {
            let report = report(gpus.clone());
            let releases: Vec<AvailableRelease> = backends
                .iter()
                .enumerate()
                .map(|(i, backend)| release(backend.clone(), &format!("b10000{}", i)))
                .collect();

            let result = select_backend(&report, &releases);
            let matches = match (&result, expected) {
                (Ok(selection), Expected::Cuda(major)) => {
                    selection.release.backend == Backend::Cuda { major: *major }
                        && selection.warning.is_none()
                }
                (Ok(selection), Expected::Vulkan) => {
                    selection.release.backend == Backend::Vulkan && selection.warning.is_some()
                }
                (Ok(selection), Expected::Cpu) => {
                    selection.release.backend == Backend::Cpu && selection.warning.is_some()
                }
                (Err(AppError::NotFound { .. }), Expected::NotFound) => true,
                _ => false,
            };
            assert!(
                matches,
                "profile: {name}: got {result:?}, expected {expected:?}"
            );
        }
    }

    /// The hard-failure message must name the silent-CPU-fallback risk, not
    /// be a downgrade or a warning-and-proceed.
    #[test]
    fn blackwell_no_cuda13_asset_fails_with_the_silent_fallback_message() {
        let report = report(vec![gpu("NVIDIA GeForce RTX 5090", (12, 0), "580.88")]);
        let releases = vec![
            release(Backend::Cuda { major: 12 }, "b1"),
            release(Backend::Vulkan, "b2"),
            release(Backend::Cpu, "b3"),
        ];

        let result = select_backend(&report, &releases);
        let Err(AppError::NotFound { what }) = result else {
            panic!("expected a hard failure, got {result:?}");
        };
        assert!(
            what.contains("silently fall back to CPU"),
            "the message must name the silent-CPU-fallback risk, got: {what}"
        );
    }

    proptest! {
        /// Property: the function never returns a CUDA build for a driver that
        /// cannot run it. A Vulkan release is always available, so a CUDA build
        /// is only returned when the driver can actually run it.
        #[test]
        fn property_never_returns_cuda_for_unrunnable_driver(
            cc_major in 0..20u32,
            driver_major in 0..700u32,
            cuda_major in 10..20u8,
        ) {
            let report = report(vec![gpu(
                "test",
                (cc_major, 0),
                &format!("{driver_major}.0.0"),
            )]);
            let releases = vec![
                release(Backend::Cuda { major: cuda_major }, "b1"),
                release(Backend::Vulkan, "b2"),
            ];
            let result = select_backend(&report, &releases);
            if let Ok(selection) = result {
                if let Backend::Cuda { major } = selection.release.backend {
                    prop_assert!(
                        driver_can_run(Some(driver_major), major),
                        "returned CUDA {major} for a driver that cannot run it \
                         (driver major {driver_major})"
                    );
                }
            }
        }
    }

    proptest! {
        /// Property: the function never returns a CUDA major below 13 when any
        /// GPU reports compute capability >= 10.0. On a Blackwell GPU, a CUDA
        /// selection is always major >= 13 (or the selection is Vulkan/CPU, or
        /// it is a hard failure).
        #[test]
        fn property_never_returns_cuda_below_13_on_blackwell(
            driver_major in 0..700u32,
            cuda_major in 10..20u8,
        ) {
            // A Blackwell GPU (CC major 12).
            let report = report(vec![gpu(
                "test",
                (12, 0),
                &format!("{driver_major}.0.0"),
            )]);
            let releases = vec![
                release(Backend::Cuda { major: cuda_major }, "b1"),
                release(Backend::Vulkan, "b2"),
            ];
            let result = select_backend(&report, &releases);
            if let Ok(selection) = result {
                if let Backend::Cuda { major } = selection.release.backend {
                    prop_assert!(
                        major >= 13,
                        "returned CUDA {major} (< 13) on a Blackwell GPU"
                    );
                }
            }
        }
    }
}
