//! Shared continuation handling for HTTP and WebSocket lists.
use crate::ws::types::Cursor;
use std::collections::HashSet;

/// Largest `limit` any paged request accepts; the server rejects more with `invalid_limit`.
pub const MAX_PAGE_SIZE: u32 = 200;

#[derive(Default)]
pub(crate) struct Continuation {
    seen: HashSet<String>,
}
impl Continuation {
    pub(crate) fn advance(&mut self, next: Option<&str>) -> Result<Option<Cursor>, InvalidPage> {
        match next {
            None => Ok(None),
            Some(id) if id.is_empty() || !self.seen.insert(id.to_owned()) => Err(InvalidPage),
            Some(id) => Ok(Some(Cursor::After { id: id.to_owned() })),
        }
    }
}
#[derive(Debug, thiserror::Error)]
#[error("page returned an empty or repeated continuation")]
pub struct InvalidPage;

/// Collect a list without exposing cursor loops to the caller.
pub async fn fetch_all<T, E, F, Fut>(
    mut load: F,
    next: impl Fn(&T) -> Option<&str>,
    append: impl Fn(&mut T, T),
) -> Result<T, E>
where
    F: FnMut(Cursor) -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: From<InvalidPage>,
{
    let mut all = load(Cursor::Start {}).await?;
    let mut continuation = Continuation::default();
    while let Some(cursor) = continuation.advance(next(&all))? {
        let page = load(cursor).await?;
        append(&mut all, page);
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Debug)]
    struct Page {
        rows: Vec<u8>,
        next: Option<String>,
    }
    fn append(all: &mut Page, page: Page) {
        all.rows.extend(page.rows);
        all.next = page.next;
    }
    #[tokio::test]
    async fn follows_opaque_continuation_and_rejects_cycles() {
        let mut requests = Vec::new();
        let result: Result<Page, InvalidPage> = fetch_all(
            |cursor| {
                requests.push(cursor.clone());
                std::future::ready(Ok(match cursor {
                    Cursor::Start {} => Page {
                        rows: vec![1],
                        next: Some("opaque".into()),
                    },
                    Cursor::After { id } => {
                        assert_eq!(id, "opaque");
                        Page {
                            rows: vec![2],
                            next: None,
                        }
                    }
                }))
            },
            |page| page.next.as_deref(),
            append,
        )
        .await;
        assert_eq!(result.unwrap().rows, [1, 2]);
        assert_eq!(requests.len(), 2);
        let result: Result<Page, InvalidPage> = fetch_all(
            |_| {
                std::future::ready(Ok(Page {
                    rows: vec![],
                    next: Some("cycle".into()),
                }))
            },
            |page| page.next.as_deref(),
            append,
        )
        .await;
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn later_failure_never_returns_a_partial_list() {
        let result: Result<Page, InvalidPage> = fetch_all(
            |cursor| {
                std::future::ready(match cursor {
                    Cursor::Start {} => Ok(Page {
                        rows: vec![1],
                        next: Some("next".into()),
                    }),
                    Cursor::After { .. } => Err(InvalidPage),
                })
            },
            |page| page.next.as_deref(),
            append,
        )
        .await;
        assert!(result.is_err());
    }
}
