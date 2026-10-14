use symphonia::core::formats::{probe::Hint, FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
fn main() {
    for p in std::env::args().skip(1) {
        let f = std::fs::File::open(&p).unwrap();
        let mss = MediaSourceStream::new(Box::new(f), MediaSourceStreamOptions::default());
        let mut h = Hint::new();
        if let Some(e) = std::path::Path::new(&p)
            .extension()
            .and_then(|e| e.to_str())
        {
            h.with_extension(e);
        }
        let mut fmt = symphonia::default::get_probe()
            .probe(
                &h,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .unwrap();
        let t = fmt.default_track(TrackType::Audio).unwrap().clone();
        let mut first = None;
        let mut n = 0;
        let mut tot_trim_s = 0u64;
        let mut tot_trim_e = 0u64;
        while let Ok(Some(pk)) = fmt.next_packet() {
            if pk.track_id != t.id {
                continue;
            }
            if first.is_none() {
                first = Some((pk.trim_start, pk.trim_end, pk.dur));
            }
            tot_trim_s += pk.trim_start.get();
            tot_trim_e += pk.trim_end.get();
            n += 1;
        }
        println!("{p}: frames={:?} dur={:?} delay={:?} padding={:?} tb={:?} start_ts={:?} packets={n} first={:?} sum_trim_start={tot_trim_s} sum_trim_end={tot_trim_e}", t.num_frames, t.duration, t.delay, t.padding, t.time_base, t.start_ts, first);
    }
}
