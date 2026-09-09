//! Snapshot and capability-aware controls for the activity screen.
use crate::{Error, Kodi, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Playback {
    pub player: i64,
    pub item: Value,
    pub properties: Value,
}
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct Chapter {
    pub index: u32,
    #[serde(default)]
    pub name: String,
    pub time: u32,
}
impl Kodi {
    pub fn playback(&self) -> Result<Option<Playback>> {
        // A video activity must not accidentally attach to a slideshow/audio player.
        let players = self.active_players()?;
        let Some(player) = players.iter().find(|p| p.kind == "video") else {
            return Ok(None);
        };
        let item = self.call(
            "Player.GetItem",
            json!({"playerid":player.id,
            "properties":["title","showtitle","season","episode","year","art","file"]}),
        )?;
        let properties = self.call(
            "Player.GetProperties",
            json!({"playerid":player.id,
            "properties":["time","totaltime","speed","canseek","live","audiostreams",
                "currentaudiostream","subtitles","currentsubtitle","subtitleenabled"]}),
        )?;
        Ok(Some(Playback {
            player: player.id,
            item: item["item"].clone(),
            properties,
        }))
    }
    pub fn chapters(&self, player: i64) -> Result<Option<Vec<Chapter>>> {
        match self.call("Player.GetChapters", json!({"playerid":player})) {
            Ok(v) => Ok(Some(serde_json::from_value(v["chapters"].clone())?)),
            Err(Error::Rpc { code: -32601, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn player_command(&self, player: i64, method: &str, mut params: Value) -> Result<Value> {
        params["playerid"] = json!(player);
        self.call(method, params)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unnamed_chapters_keep_their_real_start_times() {
        let chapter: Chapter = serde_json::from_value(json!({"index":2,"time":87})).unwrap();
        assert_eq!(chapter.time, 87);
        assert!(chapter.name.is_empty());
        assert!(serde_json::from_value::<Chapter>(json!({"index":2,"time":-1})).is_err());
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    #[test]
    fn selects_video_and_treats_missing_chapter_method_as_optional() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            for method in [
                "Player.GetActivePlayers",
                "Player.GetItem",
                "Player.GetProperties",
                "Player.GetChapters",
                "Player.Seek",
            ] {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["method"], method);
                if method != "Player.GetActivePlayers" {
                    assert_eq!(request["params"]["playerid"], 7);
                }
                let mut reply = json!({"jsonrpc":"2.0","id":request["id"]});
                if method == "Player.GetChapters" {
                    reply["error"] = json!({"code":-32601,"message":"Method not found"});
                } else {
                    reply["result"] = match method {
                        "Player.GetActivePlayers" => {
                            json!([{"playerid":2,"type":"audio"},{"playerid":7,"type":"video"}])
                        }
                        "Player.GetItem" => json!({"item":{"title":"Film","file":"movie.mkv"}}),
                        "Player.GetProperties" => json!({"speed":1,"canseek":false}),
                        _ => {
                            assert_eq!(request["params"]["value"]["seconds"], 30);
                            json!({})
                        }
                    };
                }
                writeln!(socket, "{reply}").unwrap();
            }
        });
        let client = Kodi::tcp("127.0.0.1", port);
        let state = client.playback().unwrap().unwrap();
        assert_eq!(state.player, 7);
        assert_eq!(state.item["title"], "Film");
        assert!(client.chapters(7).unwrap().is_none());
        client
            .player_command(7, "Player.Seek", json!({"value":{"seconds":30}}))
            .unwrap();
        worker.join().unwrap();
    }
}
