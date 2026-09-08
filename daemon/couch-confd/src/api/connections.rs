use super::{parse, Api, Reply};
use couch_model::{Connection, Id, Provider};
use serde::Deserialize;
#[derive(Deserialize)]
struct Setup {
    name: String,
    provider: Provider,
}
impl Api {
    pub(super) fn connection_route(
        &self,
        method: &str,
        path: &[&str],
        body: &[u8],
        revision: Option<u64>,
    ) -> Reply {
        match (method, path) {
            ("POST", []) | ("PUT", [_]) => {
                let input: Setup = match parse(body) {
                    Ok(v) => v,
                    Err(r) => return r,
                };
                if method == "PUT" {
                    let id = Id::new(path[0]);
                    if !self.with(|s| {
                        s.config()
                            .connection(&id)
                            .is_some_and(|c| c.provider.kind() == input.provider.kind())
                    }) {
                        return Reply::error(
                            400,
                            "A connection's type cannot be changed; add a new connection instead",
                        );
                    }
                    self.edit_found(revision, move |c| {
                        let slot = c.connections.iter_mut().find(|c| c.id == id)?;
                        slot.name = input.name;
                        slot.provider = input.provider;
                        Some(())
                    })
                } else {
                    self.edit(revision, move |c| {
                        let id = Id::unique(&input.name, c.connections.iter().map(|c| &c.id));
                        c.connections.push(Connection {
                            id,
                            name: input.name,
                            provider: input.provider,
                        });
                    })
                }
            }
            ("DELETE", [id]) => {
                let id = Id::new(*id);
                // Config validation rejects a deletion while devices refer to it.
                self.edit_found(revision, move |c| {
                    let at = c.connections.iter().position(|c| c.id == id)?;
                    c.connections.remove(at);
                    Some(())
                })
            }
            _ => Reply::error(404, "Unknown connection operation"),
        }
    }
}
