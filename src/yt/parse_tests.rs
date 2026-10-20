use super::*;

fn fixture(name: &str) -> Value {
    let path = format!("{}/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture exists")).expect("fixture parses")
}

fn track(item: &Item) -> &Track {
    match item {
        Item::Track(t) => t,
        other => panic!("expected a track, got {other:?}"),
    }
}

fn coll(item: &Item) -> &Collection {
    match item {
        Item::Collection(c) => c,
        other => panic!("expected a collection, got {other:?}"),
    }
}

#[test]
fn search_results() {
    let s = shelves(&fixture("search"));
    let titles: Vec<&str> = s.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Top result", "Songs", "Videos", "Albums", "Artists", "Community playlists"]);

    let top = coll(&s[0].items[0]);
    assert_eq!((top.kind, top.title.as_str()), (Kind::Artist, "Aurora Vale"));
    assert_eq!(s[0].items.len(), 3);

    let song = track(&s[1].items[0]);
    assert_eq!(song.title, "Glass Harbour");
    assert_eq!(song.video_id.as_deref(), Some("a1b2c3d4e5f"));
    assert_eq!(song.artist_line(), "Lumen");
    assert_eq!(song.album.as_ref().map(|a| a.name.as_str()), Some("Tidal Rooms"));
    assert_eq!(song.duration, Some(245));
    assert!(!song.is_video);
    assert!(song.thumbnail.as_deref().unwrap().contains("w120"));

    let duet = track(&s[1].items[1]);
    assert_eq!(duet.artist_line(), "Kai Rivers, Nova");

    assert!(track(&s[2].items[0]).is_video);

    let album = coll(&s[3].items[0]);
    assert_eq!(album.kind, Kind::Album);
    assert_eq!(album.play_id.as_deref(), Some("OLAK5uy_polar"));
    assert_eq!(coll(&s[3].items[1]).kind, Kind::Single);
    assert_eq!(coll(&s[4].items[0]).kind, Kind::Artist);
    assert_eq!(coll(&s[5].items[0]).id, "VLPLchill0001");
}

#[test]
fn home_shelves() {
    let s = shelves(&fixture("home"));
    assert_eq!(s.len(), 4);
    assert_eq!(s[0].title, "Quick picks");
    assert_eq!(s[0].items.len(), 8);
    assert_eq!(s[1].items.len(), 6);
    let mix = coll(&s[2].items[0]);
    assert_eq!(mix.kind, Kind::Playlist);
    assert_eq!(mix.play_id.as_deref(), Some("RDTMAK5uy_mix1"));
    assert_eq!(s[2].more.as_deref(), Some("FEmusic_mixed_for_you"));
    let video = track(&s[3].items[0]);
    assert!(video.is_video);
    assert_eq!(video.artist_line(), "Aurora Vale");
}

#[test]
fn album_page() {
    let d = detail(&fixture("album"), "MPREb_polar001");
    assert_eq!(d.title, "Polar Skies");
    assert!(d.subtitle.contains("Aurora Vale"));
    assert!(d.description.starts_with("Polar Skies is"));
    assert_eq!(d.tracks.len(), 6);
    // Album rows have no artist or art of their own: filled from the header.
    assert_eq!(d.tracks[2].artist_line(), "Aurora Vale");
    assert_eq!(d.tracks[2].artists[0].id.as_deref(), Some("UCaurora1111111111111111"));
    assert!(d.tracks[2].thumbnail.is_some());
    assert_eq!(d.tracks[2].album.as_ref().unwrap().id.as_deref(), Some("MPREb_polar001"));
    assert_eq!(d.playlist_id.as_deref(), Some("OLAK5uy_polar"));
    assert_eq!(d.shelves.len(), 1);
    assert!(!d.editable);
}

#[test]
fn playlist_page_and_continuation() {
    let d = detail(&fixture("playlist"), "VLPLchill0001");
    assert_eq!(d.title, "Late Night Chill");
    assert!(d.editable);
    assert_eq!(d.tracks.len(), 8);
    assert_eq!(d.tracks[0].set_video_id.as_deref(), Some("SET0000"));
    assert_eq!(d.playlist_id.as_deref(), Some("PLchill0001"));
    assert_eq!(d.continuation.as_deref(), Some("4qmFsgKCONT"));

    let more = fixture("playlist_continuation");
    let t = tracks(&more);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].title, "Coda");
    assert_eq!(continuation(&more), None);
}

#[test]
fn artist_page() {
    let d = detail(&fixture("artist"), "UCaurora1111111111111111");
    assert_eq!(d.title, "Aurora Vale");
    assert_eq!(d.tracks.len(), 3);
    assert_eq!(d.shelves.len(), 4);
    assert_eq!(d.shelves[1].title, "Albums");
    assert_eq!(d.radio_id.as_deref(), Some("RDEMaurora"));
    assert_eq!(d.shelves[0].more.as_deref(), Some("VLOLAK5uy_aurora_top"));
}

#[test]
fn watch_queue_and_lyrics() {
    let (queue, lyrics_id) = next(&fixture("next"));
    assert_eq!(queue.len(), 6);
    assert_eq!(queue[0].artist_line(), "Aurora Vale");
    assert_eq!(queue[0].album.as_ref().unwrap().name, "Polar Skies");
    assert_eq!(queue[0].duration, Some(221));
    assert_eq!(lyrics_id.as_deref(), Some("MPLYt_northern"));
    let l = lyrics(&fixture("lyrics")).unwrap();
    assert!(l.text.starts_with("Under the northern lights"));
    assert_eq!(l.source, "Source: LyricFind");
}

#[test]
fn library_and_account() {
    let pl = items(&fixture("library_playlists"));
    // The "New playlist" tile has no browse target and is skipped.
    assert_eq!(pl.len(), 4);
    assert_eq!(coll(&pl[0]).id, "VLLM");
    let artists = items(&fixture("library_artists"));
    assert_eq!(coll(&artists[0]).id, "UCaurora1111111111111111");
    assert_eq!(coll(&artists[0]).kind, Kind::Artist);
    let a = account(&fixture("account")).unwrap();
    assert_eq!((a.name.as_str(), a.handle.as_str()), ("Bhima", "@bhima"));
    assert_eq!(
        suggestions(&fixture("suggestions")),
        ["aurora vale", "aurora vale northern lights", "aurora borealis"]
    );
}

#[test]
fn garbage_does_not_panic() {
    for v in [serde_json::json!(null), serde_json::json!([]), serde_json::json!({"contents": 3}),
              serde_json::json!({"musicResponsiveListItemRenderer": {"flexColumns": "x"}})] {
        assert!(shelves(&v).is_empty());
        let _ = detail(&v, "VLx");
        let _ = next(&v);
        assert!(lyrics(&v).is_none());
    }
}
