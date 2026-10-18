#!/usr/bin/env python3
"""Generate Parlo test fixtures (BRIEF §6.2, §14.1). OS-aware: espeak-ng (Linux) or `say` (macOS) for synthetic
speech; ffmpeg for formats/noise/long files; FLEURS subset via scripts/fleurs_subset.py.

Groups (--only a,b,...):  synth  formats  edge  text  url  fleurs   (default: synth,formats,edge,text,url)
Outputs:
  fixtures/generated/   (gitignored)  audio fixtures + refs.jsonl (reference transcripts)
  fixtures/text/        (committed, tiny) text-import inputs; expected snapshots are added by the importer tests
Flags: --long2h also builds a 2-hour 16 kHz mono WAV (~230 MB) for bounded-memory tests (gitignored).
"""
import argparse, json, os, platform, shutil, subprocess, sys, textwrap, zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GEN = ROOT / "fixtures" / "generated"
TXT = ROOT / "fixtures" / "text"
IS_MAC = platform.system() == "Darwin"

# language -> (espeak voice, macOS `say` voice, sentences)
SENTENCES = {
    "en": ("en-us", "Samantha", [
        "The weather today is sunny with a light breeze from the west.",
        "Please remember to submit your report before the end of the week.",
        "Yesterday we walked along the river and talked about our plans for the summer.",
    ]),
    "de": ("de", "Anna", [
        "Das Wetter ist heute sonnig, und es weht ein leichter Wind aus Westen.",
        "Bitte denken Sie daran, Ihren Bericht bis Ende der Woche einzureichen.",
        "Gestern sind wir am Fluss entlanggegangen und haben über unsere Pläne gesprochen.",
    ]),
    "fr": ("fr-fr", "Thomas", [
        "Il fait beau aujourd'hui, avec une légère brise venant de l'ouest.",
        "N'oubliez pas de remettre votre rapport avant la fin de la semaine.",
        "Hier, nous avons marché le long de la rivière en parlant de nos projets d'été.",
    ]),
    "es": ("es", "Monica", [
        "Hoy hace sol y sopla una suave brisa del oeste.",
        "Por favor, recuerde entregar su informe antes del final de la semana.",
        "Ayer caminamos junto al río y hablamos de nuestros planes para el verano.",
    ]),
    "it": ("it", "Alice", [
        "Oggi il tempo è soleggiato, con una leggera brezza da ovest.",
        "Ricordati di consegnare la relazione prima della fine della settimana.",
        "Ieri abbiamo camminato lungo il fiume parlando dei nostri progetti per l'estate.",
    ]),
    "bg": ("bg", None, [
        "Днес времето е слънчево с лек ветрец от запад.",
        "Моля, не забравяйте да предадете доклада си до края на седмицата.",
        "Вчера се разходихме край реката и говорихме за плановете си за лятото.",
    ]),
    "ru": ("ru", "Milena", [
        "Сегодня солнечная погода, с запада дует лёгкий ветер.",
        "Пожалуйста, не забудьте сдать отчёт до конца недели.",
        "Вчера мы гуляли вдоль реки и говорили о наших планах на лето.",
    ]),
    "pt": ("pt-br", "Luciana", [
        "Hoje o tempo está ensolarado, com uma brisa leve vinda do oeste.",
        "Lembre-se de entregar o seu relatório antes do fim da semana.",
        "Ontem caminhamos ao longo do rio e conversamos sobre os nossos planos para o verão.",
    ]),
}
LONG_EN = (
    "Speech recognition has improved dramatically over the last decade. Models that once needed a room full of "
    "servers now run comfortably on a laptop. This makes it possible to transcribe interviews, lectures and "
    "meetings without sending a single word to the cloud. Privacy is the obvious benefit, but there is another: "
    "reliability. A tool that works on a train, on a plane, or in a basement with no signal is a tool you can "
    "depend on. The trade-off is that the user's own hardware must do the work, so efficiency matters. "
    "Careful engineering can keep memory use low and let the computer return to rest as soon as the job is done."
)

def run(cmd, **kw):
    r = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, **kw)
    if r.returncode != 0:
        raise RuntimeError(f"{' '.join(map(str, cmd))}\n{r.stderr[-800:]}")
    return r

def ff(*args):
    return run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *map(str, args)])

def synth(lang, text, out_wav: Path):
    """Synthetic speech to a WAV (native rate)."""
    esp, say_voice, _ = SENTENCES.get(lang, ("en-us", "Samantha", []))
    if IS_MAC and say_voice:
        aiff = out_wav.with_suffix(".aiff")
        run(["say", "-v", say_voice, "-o", str(aiff), text])
        ff("-i", aiff, "-ac", "1", out_wav); aiff.unlink()
    else:
        run(["espeak-ng", "-v", esp, "-s", "150", "-w", str(out_wav), text])

def dur(p: Path) -> float:
    r = run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(p)])
    return float(r.stdout.strip())

def g_synth():
    d = GEN / "synth"; d.mkdir(parents=True, exist_ok=True)
    refs = []
    for lang, (_, _, sents) in SENTENCES.items():
        for i, s in enumerate(sents):
            wav = d / f"{lang}_{i+1}.wav"
            if not wav.exists():
                synth(lang, s, wav)
            refs.append(dict(file=f"synth/{wav.name}", lang=lang, text=s, duration_s=round(dur(wav), 2)))
        # whole-language file: the three sentences with 0.8 s pauses
        joined = d / f"{lang}_all.wav"
        if not joined.exists():
            parts = [d / f"{lang}_{i+1}.wav" for i in range(len(sents))]
            concat(parts, joined, gap_s=0.8)
        refs.append(dict(file=f"synth/{joined.name}", lang=lang, text=" ".join(sents), duration_s=round(dur(joined), 2)))
    long_en = d / "en_long_paragraph.wav"
    if not long_en.exists():
        synth("en", LONG_EN, long_en)
    refs.append(dict(file="synth/en_long_paragraph.wav", lang="en", text=LONG_EN, duration_s=round(dur(long_en), 2)))
    (d / "refs.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in refs))
    print(f"synth: {len(refs)} files")

def concat(parts, out: Path, gap_s=0.0, rate=22050):
    """Concatenate WAVs (re-encoded to a common format) with silence gaps."""
    lst = out.with_suffix(".txt")
    sil = out.with_suffix(".sil.wav")
    if gap_s:
        ff("-f", "lavfi", "-i", f"anullsrc=r={rate}:cl=mono", "-t", gap_s, "-c:a", "pcm_s16le", sil)
    lines = []
    for i, p in enumerate(parts):
        norm = out.with_suffix(f".p{i}.wav")
        ff("-i", p, "-ar", rate, "-ac", 1, "-c:a", "pcm_s16le", norm)
        lines.append(f"file '{norm}'")
        if gap_s and i < len(parts) - 1:
            lines.append(f"file '{sil}'")
    lst.write_text("\n".join(lines) + "\n")
    ff("-f", "concat", "-safe", 0, "-i", lst, "-c", "copy", out)
    for f in out.parent.glob(out.stem + ".p*.wav"):
        f.unlink()
    lst.unlink(); sil.unlink(missing_ok=True)

def g_formats():
    """Every input format of BRIEF §7.1 (audio) + video containers, from one 22 s English source."""
    src = GEN / "synth" / "en_all.wav"
    if not src.exists():
        g_synth()
    d = GEN / "formats"; d.mkdir(parents=True, exist_ok=True)
    audio = {
        "wav_pcm16_44k_stereo.wav": ["-ar", 44100, "-ac", 2, "-c:a", "pcm_s16le"],
        "wav_pcm16_16k_mono.wav": ["-ar", 16000, "-ac", 1, "-c:a", "pcm_s16le"],
        "wav_pcm16_8k_mono.wav": ["-ar", 8000, "-ac", 1, "-c:a", "pcm_s16le"],
        "wav_pcm24_48k_mono.wav": ["-ar", 48000, "-ac", 1, "-c:a", "pcm_s24le"],
        "wav_float32_48k_stereo.wav": ["-ar", 48000, "-ac", 2, "-c:a", "pcm_f32le"],
        "mp3_128k.mp3": ["-ar", 44100, "-c:a", "libmp3lame", "-b:a", "128k"],
        "mp3_vbr_mono.mp3": ["-ar", 22050, "-ac", 1, "-c:a", "libmp3lame", "-q:a", 4],
        "aac_128k.m4a": ["-ar", 44100, "-c:a", "aac", "-b:a", "128k"],
        "alac.m4a": ["-ar", 44100, "-c:a", "alac"],
        "flac.flac": ["-ar", 44100, "-c:a", "flac"],
        "vorbis.ogg": ["-ar", 44100, "-c:a", "libvorbis", "-q:a", 4],
        "opus.ogg": ["-ar", 48000, "-c:a", "libopus", "-b:a", "48k"],
        "opus.webm": ["-ar", 48000, "-c:a", "libopus", "-b:a", "48k"],
        "aiff_pcm16.aiff": ["-ar", 44100, "-c:a", "pcm_s16be"],
        "caf_pcm16.caf": ["-ar", 44100, "-c:a", "pcm_s16le"],
        "caf_alac.caf": ["-ar", 44100, "-c:a", "alac"],
    }
    for name, args in audio.items():
        out = d / name
        if not out.exists():
            ff("-i", src, *args, out)
    video = {  # video container + (tiny) test video + audio track
        "video_h264_aac.mp4": ["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"],
        "video_h264_aac.mov": ["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"],
        "video_h264_aac.m4v": ["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-f", "mp4"],
        "video_h264_aac.mkv": ["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"],
        "video_vp9_opus.webm": ["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", 8, "-c:a", "libopus"],
    }
    for name, args in video.items():
        out = d / name
        if not out.exists():
            ff("-f", "lavfi", "-i", "color=c=0x336699:s=160x120:r=10", "-i", src, "-shortest", "-pix_fmt", "yuv420p", *args, out)
    # error cases (BRIEF §7.1: reject with a clear message)
    ff("-f", "lavfi", "-i", "color=c=black:s=160x120:r=10", "-t", 2, "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", d / "novideo_noaudio.mp4")
    (d / "empty.wav").write_bytes(b"")
    (d / "garbage.mp3").write_bytes(os.urandom(4096))
    ff("-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono", "-t", 0.0, "-c:a", "pcm_s16le", d / "zero_duration.wav")
    print(f"formats: {len(audio)} audio + {len(video)} video + 4 error cases")

def g_edge():
    d = GEN / "edge"; d.mkdir(parents=True, exist_ok=True)
    s = GEN / "synth"
    if not (s / "en_all.wav").exists():
        g_synth()
    # silence-only (60 s) and low-level noise-only (60 s): must yield empty transcripts (no hallucinations)
    ff("-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono", "-t", 60, "-c:a", "pcm_s16le", d / "silence_60s.wav")
    ff("-f", "lavfi", "-i", "anoisesrc=d=60:c=pink:r=16000:a=0.02", "-ac", 1, "-c:a", "pcm_s16le", d / "noise_pink_60s.wav")
    ff("-f", "lavfi", "-i", "sine=f=440:r=16000:d=30", "-ac", 1, "-c:a", "pcm_s16le", d / "tone_30s.wav")
    # speech with long silences + background noise: [speech][20 s silence][speech][noise-only 20 s][speech]
    parts = [s / "en_1.wav", s / "en_2.wav", s / "en_3.wav"]
    noisy = d / "speech_silence_noise.wav"
    if not noisy.exists():
        tmp = d / "_tmp"; tmp.mkdir(exist_ok=True)
        norm = []
        for i, p in enumerate(parts):
            n = tmp / f"p{i}.wav"; ff("-i", p, "-ar", 16000, "-ac", 1, "-c:a", "pcm_s16le", n); norm.append(n)
        sil = tmp / "sil20.wav"; ff("-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono", "-t", 20, "-c:a", "pcm_s16le", sil)
        nz = tmp / "noise20.wav"; ff("-f", "lavfi", "-i", "anoisesrc=d=20:c=brown:r=16000:a=0.03", "-ac", 1, "-c:a", "pcm_s16le", nz)
        order = [norm[0], sil, norm[1], nz, norm[2]]
        (tmp / "l.txt").write_text("".join(f"file '{p}'\n" for p in order))
        mixed = tmp / "mix.wav"; ff("-f", "concat", "-safe", 0, "-i", tmp / "l.txt", "-c", "copy", mixed)
        # add faint background hum under everything
        ff("-i", mixed, "-f", "lavfi", "-i", "anoisesrc=c=pink:r=16000:a=0.01", "-filter_complex", "amix=inputs=2:duration=first:normalize=0", "-c:a", "pcm_s16le", noisy)
        shutil.rmtree(tmp)
    # mixed-language file: en, de, fr, es back to back
    mixed_lang = d / "mixed_en_de_fr_es.wav"
    if not mixed_lang.exists():
        concat([s / f"{l}_all.wav" for l in ("en", "de", "fr", "es")], mixed_lang, gap_s=1.5, rate=16000)
    # >=10 min English: loop the English clips (synthetic + FLEURS English if present) with 1 s gaps
    long10 = d / "en_long_10min.wav"
    if not long10.exists():
        sources = [s / "en_all.wav", s / "en_long_paragraph.wav"]
        fl = sorted((GEN / "fleurs" / "en_us").glob("*.wav")) if (GEN / "fleurs").exists() else []
        sources += fl
        seq, total = [], 0.0
        while total < 615:
            for p in sources:
                seq.append(p); total += dur(p) + 1.0
        concat(seq, long10, gap_s=1.0, rate=16000)
    print("edge: silence, noise, tone, speech_silence_noise, mixed_lang, en_long_10min")

def g_long2h(args):
    d = GEN / "edge"; d.mkdir(parents=True, exist_ok=True)
    out = d / "en_2h.wav"
    if out.exists():
        return
    src = d / "en_long_10min.wav"
    if not src.exists():
        g_edge()
    # 2 h = 12 x the 10 min file, streamed by ffmpeg (stream_loop), no big RAM
    ff("-stream_loop", 11, "-i", src, "-t", 7200, "-c:a", "pcm_s16le", out)
    print(f"long2h: {out} ({out.stat().st_size/1e6:.0f} MB)")

# ----------------------------------------------------------------------------------------- text fixtures
PLAIN = ("Parlo reads text aloud.\n\nIt handles accents like café, naïve, and Zürich; quotes “like these” and ‘these’; "
         "dashes — long and – short; and the euro sign (€5).\n\nSecond paragraph: numbers 1,250.75 and dates like 3rd of March.\n")
CYR = "Парло чете текст на глас.\n\nТова е втори абзац с кирилица: здравей, свят! Днес е хубав ден.\n"

MD = textwrap.dedent("""\
    # The Sample Document

    A short *introduction* with **bold** text, `inline code`, and a [link to the docs](https://example.com/docs).
    Bare URL: https://example.com/page?x=1&y=2 should become "link".

    ## Shopping list

    - Two apples
    - Three oranges, ripe
    - A loaf of [fresh bread](https://example.com/bread)

    1. First step
    2. Second step
    3. Third step

    ### Code

    ```python
    def hello():
        print("this block is skipped by default")
    ```

    > A quoted line that stays as text.

    | Name | Qty |
    |------|-----|
    | Tea  | 2   |

    ![An image](image.png)

    Final paragraph.
    """)

PDF_PAGES = [
    ["ACME Corp — Annual Report 2025", "", "1. Introduction", "This report summarises the fi-", "nancial results of the past year and the",
     "organisation's plans for the next twelve", "months. Revenue grew by 12 percent, driven by", "strong demand in the informa-", "tion services segment.", "", "Page 1 of 4"],
    ["ACME Corp — Annual Report 2025", "", "2. Results", "Operating costs rose only slightly, which", "improved margins considerably. Customer sat-",
     "isfaction remained high across all regions.", "", "The following table is omitted.", "", "Page 2 of 4"],
    ["ACME Corp — Annual Report 2025", "", "3. Outlook", "We expect continued growth, subject to the", "usual market risks and regulatory changes.", "", "Page 3 of 4"],
    ["ACME Corp — Annual Report 2025", "", "4. Appendix", "Additional notes appear here.", "", "Page 4 of 4"],
]

def g_text():
    TXT.mkdir(parents=True, exist_ok=True)
    (TXT / "plain_utf8.txt").write_text(PLAIN, encoding="utf-8")
    (TXT / "plain_utf8_bom.txt").write_bytes(b"\xef\xbb\xbf" + PLAIN.encode("utf-8"))
    (TXT / "plain_utf16le_bom.txt").write_bytes(b"\xff\xfe" + PLAIN.encode("utf-16-le"))
    (TXT / "plain_utf16be_bom.txt").write_bytes(b"\xfe\xff" + PLAIN.encode("utf-16-be"))
    (TXT / "plain_cp1252.txt").write_bytes(PLAIN.encode("cp1252", errors="replace"))
    (TXT / "plain_latin1.txt").write_bytes("Café crème, naïve façade, Zürich. Les élèves écoutent la leçon avec attention.\n".encode("latin-1"))
    (TXT / "plain_cp1251.txt").write_bytes(CYR.encode("cp1251"))
    (TXT / "plain_utf8_cyrillic.txt").write_text(CYR, encoding="utf-8")
    (TXT / "sample.md").write_text(MD, encoding="utf-8")
    json.dump({"source": "synthetic", "pages": ["\n".join(p) for p in PDF_PAGES]}, (TXT / "pdf_pages_headers_hyphens.json").open("w"), indent=1, ensure_ascii=False)
    json.dump({"source": "synthetic scanned pdf (no text layer)", "pages": ["", "", ""]}, (TXT / "pdf_pages_scanned.json").open("w"), indent=1)
    make_docx(TXT / "sample.docx")
    make_epub(TXT / "sample.epub")
    make_pdfs()
    print("text fixtures written to fixtures/text/")

def make_docx(path: Path):
    import docx
    d = docx.Document()
    d.sections[0].header.paragraphs[0].text = "Header text that must be skipped"
    d.sections[0].footer.paragraphs[0].text = "Footer text that must be skipped"
    d.add_heading("Sample Word Document", level=0)
    d.add_paragraph("This paragraph has ").add_run("bold").bold = True
    d.paragraphs[-1].add_run(" and plain text, plus a hyphen-ated word and an em dash — like this.")
    d.add_heading("A section", level=1)
    d.add_paragraph("First list item", style="List Bullet")
    d.add_paragraph("Second list item", style="List Bullet")
    t = d.add_table(rows=2, cols=2)
    t.cell(0, 0).text, t.cell(0, 1).text, t.cell(1, 0).text, t.cell(1, 1).text = "Name", "Qty", "Tea", "2"
    d.add_paragraph("Closing paragraph after the table.")
    d.save(path)

def make_epub(path: Path):
    xhtml = lambda title, body: f'<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>{title}</title></head><body>{body}</body></html>'
    files = {
        "mimetype": "application/epub+zip",
        "META-INF/container.xml": '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>',
        "OEBPS/content.opf": '''<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">parlo-test</dc:identifier><dc:title>Sample Book</dc:title><dc:language>en</dc:language></metadata>
<manifest><item id="ch3" href="ch3.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest>
<spine toc="ncx"><itemref idref="nav" linear="no"/><itemref idref="ch1"/><itemref idref="ch2"/><itemref idref="ch3"/></spine></package>''',
        "OEBPS/nav.xhtml": xhtml("Contents", '<nav epub:type="toc"><h1>Contents</h1><ol><li><a href="ch1.xhtml">Chapter One</a></li><li><a href="ch2.xhtml">Chapter Two</a></li><li><a href="ch3.xhtml">Chapter Three</a></li></ol></nav>'),
        "OEBPS/toc.ncx": '<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap><navPoint id="n1" playOrder="1"><navLabel><text>Chapter One</text></navLabel><content src="ch1.xhtml"/></navPoint></navMap></ncx>',
        "OEBPS/ch1.xhtml": xhtml("Chapter One", "<h1>Chapter One</h1><p>It began on a quiet Tuesday &amp; nobody noticed.</p><p>The rain fell <em>softly</em> on the empty street.</p>"),
        "OEBPS/ch2.xhtml": xhtml("Chapter Two", "<h1>Chapter Two</h1><p>Morning came&#8212;slowly&#8212;and with it a letter.</p><ul><li>First clue</li><li>Second clue</li></ul><script>var x = 'skipped';</script>"),
        "OEBPS/ch3.xhtml": xhtml("Chapter Three", "<h1>Chapter Three</h1><p>&#8220;Is that all?&#8221; she asked. &#8220;For now,&#8221; he said.</p>"),
    }
    with zipfile.ZipFile(path, "w") as z:
        z.writestr(zipfile.ZipInfo("mimetype"), files["mimetype"], compress_type=zipfile.ZIP_STORED)
        for n, c in files.items():
            if n != "mimetype":
                z.writestr(n, c, compress_type=zipfile.ZIP_DEFLATED)

def make_pdfs():
    """Real PDFs for the Mac PDFKit tests (and cross-checking page-text fixtures)."""
    from reportlab.lib.pagesizes import A4
    from reportlab.pdfgen import canvas
    c = canvas.Canvas(str(TXT / "headers_hyphens.pdf"), pagesize=A4, invariant=1)
    for lines in PDF_PAGES:
        y = 800
        for ln in lines:
            c.drawString(60, y, ln); y -= 16
        c.showPage()
    c.save()
    # scanned: a page containing only a rasterised drawing (no text layer)
    from reportlab.lib.utils import ImageReader
    import io
    try:
        from PIL import Image, ImageDraw
        im = Image.new("L", (300, 400), 255)
        ImageDraw.Draw(im).rectangle((20, 20, 280, 380), outline=0, width=3)
        buf = io.BytesIO(); im.save(buf, "PNG"); buf.seek(0)
        c = canvas.Canvas(str(TXT / "scanned.pdf"), pagesize=A4, invariant=1)
        c.drawImage(ImageReader(buf), 60, 300, 300, 400); c.showPage(); c.save()
    except ImportError:
        print("PIL missing: scanned.pdf skipped", file=sys.stderr)

def g_url():
    """Audio shaped like what "Transcribe from URL" downloads (served by scripts/fixture-server.py in the tests):
    a podcast MP3 (ID3 + VBR-ish), a plain AAC m4a, and the two YouTube audio-only DASH shapes — fragmented MP4/AAC
    (itag 140, `moof` boxes, no `moov` sample tables) and WebM/Opus (itag 251, Cues + unknown-size friendly)."""
    src = GEN / "synth" / "en_all.wav"
    if not src.exists():
        g_synth()
    d = GEN / "url"; d.mkdir(parents=True, exist_ok=True)
    specs = {
        "podcast_ep.mp3": ["-ar", 44100, "-ac", 1, "-c:a", "libmp3lame", "-b:a", "64k", "-id3v2_version", 3, "-metadata", "title=Episode 12: Local first"],
        "episode.m4a": ["-ar", 44100, "-ac", 1, "-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart"],
        "yt_dash_140.m4a": ["-ar", 44100, "-ac", 2, "-c:a", "aac", "-b:a", "128k", "-f", "mp4",
                            "-movflags", "frag_keyframe+empty_moov+default_base_moof"],
        "yt_dash_251.webm": ["-ar", 48000, "-ac", 2, "-c:a", "libopus", "-b:a", "128k", "-f", "webm", "-dash", 1],
    }
    for name, args in specs.items():
        out = d / name
        if not out.exists():
            ff("-i", src, *args, out)
    print(f"url: {len(specs)} files")

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", default="synth,formats,edge,text,url")
    ap.add_argument("--long2h", action="store_true")
    a = ap.parse_args()
    groups = a.only.split(",")
    for tool in ("ffmpeg", "ffprobe"):
        if not shutil.which(tool):
            sys.exit(f"{tool} not found (dev dependency; brew install ffmpeg / apt install ffmpeg)")
    if "synth" in groups or "formats" in groups or "edge" in groups or "url" in groups:
        if not IS_MAC and not shutil.which("espeak-ng"):
            sys.exit("espeak-ng not found (apt install espeak-ng)")
    GEN.mkdir(parents=True, exist_ok=True)
    if "synth" in groups: g_synth()
    if "fleurs" in groups:
        subprocess.check_call(["uv", "run", "--no-project", "--with", "pyarrow", "--with", "huggingface_hub", "--with", "soundfile", "--with", "numpy", "--with", "scipy", "python", "-I", str(ROOT / "scripts/fleurs_subset.py")])
    if "formats" in groups: g_formats()
    if "url" in groups: g_url()
    if "edge" in groups: g_edge()
    if a.long2h: g_long2h(a)
    if "text" in groups: g_text()

if __name__ == "__main__":
    main()
