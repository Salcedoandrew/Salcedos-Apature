#!/usr/bin/env bash
# Download one small sample per RAW format from raw.pixls.us (CC0) into this
# folder. The files are gitignored; the sample tests skip formats whose file
# is missing. Run from anywhere: tests/raw-samples/fetch.sh
set -euo pipefail
cd "$(dirname "$0")"
base=https://raw.pixls.us/data
while read -r name path; do
  [[ -z "$name" || "$name" == \#* ]] && continue
  [[ -s "$name" ]] && continue
  curl -sfL --retry 2 -o "$name.part" "$base/${path// /%20}" && mv "$name.part" "$name" && echo "got $name"
done <<'LIST'
cr2.CR2 Canon/EOS 40D/_MG_0154.CR2
cr3.CR3 Canon/EOS R6/Canon_EOS_R6_CRAW_ISO_100_crop_nodual.CR3
crw.CRW Canon/EOS 10D/CRW_7673.CRW
nef.NEF Nikon/D70s/RAW_NIKON_D70S.NEF
nrw.NRW Nikon/Coolpix P7000/RAW_NIKON_P7000.NRW
arw.ARW Sony/ILCE-7S/DSC04126.ARW
srf.SRF Sony/DSC-F828/DSC06227.SRF
sr2.SR2 Sony/DSC-R1/_DSC1477.SR2
raf.RAF Fujifilm/X-T3/AFXT2721.RAF
raf-s5000.RAF Fujifilm/FinePix S5000/2007_0912Image0022.RAF
rw2.RW2 Panasonic/DMC-LX7/P1020372.RW2
rwl.RWL Leica/D-LUX 4/L1010119.RWL
orf.ORF Olympus/E-M10/P3180165.ORF
orf-e10.ORF Olympus/E-10/P7198607.ORF
pef.PEF Pentax/K10D/_IGP7284.PEF
dng-pentax.DNG Pentax/K-x/IMGP6431.DNG
dng.DNG Adobe DNG Converter/Canon EOS 5D Mark III/5G4A9395-compressed-lossy.DNG
dng-proraw.DNG Apple/iPhone 12 Pro/IMG_1361.DNG
3fr.3FR Hasselblad/CFV/RAW_HASSELBLAD_CFV.3FR
fff.FFF Hasselblad/H5D-40/20151114_CCSG_on_Hasselblad_H5D40-0726.fff
iiq.IIQ Phase One/P25+/CF028662.IIQ
iiq-iq150.IIQ Phase One/IQ150/CF006803.IIQ
3fr-x1d.3FR Hasselblad/X1D/B9999910.3FR
fff-x1d2.FFF Hasselblad/X1D II 50C/x1d-II-sample-02.fff
srw.SRW Samsung/NX100/RAW_SAMSUNG_NX100.SRW
x3f.X3F Sigma/DP1/RAW_SIGMA_DP1.X3F
LIST
