//! Checks the preview extractor against real camera files from raw.pixls.us.
//!
//! The files are not in git: run `tests/raw-samples/fetch.sh` first (or set
//! APATURE_RAW_SAMPLES to another folder). Missing files are skipped.

use std::path::PathBuf;

use apature_preview::{decode_jpeg, extract, list_all, Prefer, Source, DEFAULT_MAX_BYTES};

/// (file, expected largest preview width x height)
const EXPECTED: &[(&str, u32, u32)] = &[
    ("cr2.CR2", 1936, 1288),
    ("cr3.CR3", 3408, 2272),
    ("crw.CRW", 1536, 1024),
    ("nef.NEF", 3008, 2000),
    ("nrw.NRW", 3648, 2736),
    ("arw.ARW", 1616, 1080),
    ("srf.SRF", 640, 480),
    ("sr2.SR2", 640, 424),
    ("raf.RAF", 4416, 2944),
    ("raf-s5000.RAF", 1280, 960),
    ("rw2.RW2", 1920, 1920),
    ("rwl.RWL", 1920, 1280),
    ("orf.ORF", 3200, 2400),
    ("orf-e10.ORF", 160, 120),
    ("pef.PEF", 3872, 2592),
    ("dng-pentax.DNG", 4288, 2848),
    ("dng.DNG", 3960, 2640),
    ("dng-proraw.DNG", 4032, 3024),
    ("3fr.3FR", 218, 218),
    ("3fr-x1d.3FR", 1440, 1080),
    ("fff.FFF", 1217, 913),
    ("fff-x1d2.FFF", 1181, 885),
    ("iiq.IIQ", 300, 220),
    ("iiq-iq150.IIQ", 588, 442),
    ("srw.SRW", 4592, 3056),
    ("x3f.X3F", 2640, 1760),
];

fn samples() -> PathBuf {
    std::env::var_os("APATURE_RAW_SAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/raw-samples")
        })
}

#[test]
fn largest_preview_per_format() {
    let dir = samples();
    let mut checked = 0;
    for &(name, w, h) in EXPECTED {
        let path = dir.join(name);
        if !path.exists() {
            eprintln!("skip {name}: not downloaded");
            continue;
        }
        let src = Source::open(&path).unwrap();
        let c = src
            .largest()
            .unwrap_or_else(|| panic!("{name}: no preview"));
        assert_eq!(
            (c.width, c.height),
            (w, h),
            "{name}: {:?}",
            src.candidates()
        );
        // Every candidate decodes, to the size its header claims.
        for c in src.candidates() {
            let img = decode_jpeg(src.bytes(c), None).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                (img.width, img.height),
                (c.width, c.height),
                "{name} {}",
                c.source
            );
        }
        // The path-level API agrees.
        assert_eq!(list_all(&path).unwrap().len(), src.candidates().len());
        let (small, bytes) = extract(&path, Prefer::Smallest, usize::MAX).unwrap();
        assert_eq!(small.length, bytes.len());
        assert!(small.long_side() <= c.long_side());
        checked += 1;
    }
    eprintln!("checked {checked} of {} samples", EXPECTED.len());
}

#[test]
fn size_cap_skips_huge_previews() {
    let path = samples().join("dng-proraw.DNG");
    if !path.exists() {
        return;
    }
    // ProRAW's 5.4 MB full-size preview fits the default cap but not a 1 MB one.
    let (c, _) = extract(&path, Prefer::Largest, DEFAULT_MAX_BYTES).unwrap();
    assert_eq!(c.long_side(), 4032);
    assert!(extract(&path, Prefer::Largest, 1 << 20).is_err());
}
