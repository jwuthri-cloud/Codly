"""Paper search for the voice's quick lookups: OpenAlex and arXiv (from voice-research-assistant).

All calls are plain HTTPS GETs to fixed public hosts with no key; the model's words only reach them
as URL-encoded query parameters. arXiv rate-limits shared cloud IPs hard, so it falls back to
OpenAlex's copy of arXiv.
"""
import time
import xml.etree.ElementTree as ET

import httpx

USER_AGENT = "codly-voice"
TIMEOUT = 30.0
RETRIES = 4
OPENALEX = "https://api.openalex.org/works"
OPENALEX_ARXIV_SOURCE = "S4306400194"  # "arXiv (Cornell University)"
OPENALEX_SELECT = "id,title,publication_year,primary_location,authorships,cited_by_count,doi,abstract_inverted_index"


def _get(url: str, retries: int = RETRIES, **params) -> httpx.Response:
    """GET with retries on 429/503 and timeouts."""
    for attempt in range(retries):
        try:
            resp = httpx.get(url, params=params or None, headers={"User-Agent": USER_AGENT},
                             timeout=TIMEOUT, follow_redirects=True)
        except httpx.TimeoutException:
            if attempt == retries - 1:
                raise RuntimeError(f"{httpx.URL(url).host} timed out; try again") from None
            continue
        if resp.status_code not in (429, 503) or attempt == retries - 1:
            break
        wait = resp.headers.get("retry-after", "")
        time.sleep(min(float(wait), 10.0) if wait.isdigit() else 1.5 * 2 ** attempt)
    if resp.status_code == 429:
        raise RuntimeError(f"{httpx.URL(url).host} is rate-limiting us; try again in a minute")
    resp.raise_for_status()
    return resp


def _paper(title, authors, year, venue, citations=None, url=None, arxiv=None, abstract=None):
    """Common paper shape: the web app renders any list of these as paper cards."""
    out = {"title": title, "authors": authors[:3], "year": year, "venue": venue,
           "citations": citations, "url": url, "arxiv": arxiv}
    if abstract:
        out["abstract"] = abstract
    return out


def _abstract(inverted: dict | None, limit: int = 1200) -> str | None:
    if not inverted:
        return None
    words = sorted((i, w) for w, idx in inverted.items() for i in idx)
    return " ".join(w for _, w in words)[:limit]


def _oa_paper(w: dict) -> dict:
    doi = w.get("doi") or ""
    arxiv = doi.lower().split("10.48550/arxiv.", 1)[1] if "10.48550/arxiv." in doi.lower() else None
    return _paper(
        w.get("title"),
        [a["author"]["display_name"] for a in w.get("authorships", [])],
        w.get("publication_year"),
        ((w.get("primary_location") or {}).get("source") or {}).get("display_name"),
        w.get("cited_by_count"),
        f"https://arxiv.org/abs/{arxiv}" if arxiv else (doi or w.get("id")),
        arxiv,
        _abstract(w.get("abstract_inverted_index")),
    )


def _oa_works(limit: int, **params) -> list[dict]:
    data = _get(OPENALEX, **{"per-page": max(1, min(int(limit), 25)), "select": OPENALEX_SELECT, **params}).json()
    return [_oa_paper(w) for w in data["results"]]


def search_papers(query: str, limit: int = 5) -> list[dict]:
    return _oa_works(min(int(limit), 10), search=query)


def arxiv_search(query: str, limit: int = 5, sort: str = "relevance") -> list[dict]:
    limit = max(1, min(int(limit), 10))
    sort_by = {"relevance": "relevance", "recent": "submittedDate"}.get(sort, "relevance")
    try:
        xml = _get("https://export.arxiv.org/api/query", retries=2, search_query=f"all:{query}",
                   max_results=limit, sortBy=sort_by, sortOrder="descending").text
    except RuntimeError:  # rate-limited: OpenAlex's copy of arXiv, ranked by relevance
        flt = f"primary_location.source.id:{OPENALEX_ARXIV_SOURCE}"
        if sort == "recent":
            flt += ",from_publication_date:" + time.strftime("%Y-%m-%d", time.gmtime(time.time() - 365 * 86400))
        return _oa_works(limit, search=query, filter=flt)
    ns = {"a": "http://www.w3.org/2005/Atom"}
    out = []
    for e in ET.fromstring(xml).findall("a:entry", ns):
        arxiv_id = e.findtext("a:id", "", ns).rsplit("/abs/", 1)[-1]
        out.append(_paper(
            " ".join(e.findtext("a:title", "", ns).split()),
            [a.findtext("a:name", "", ns) for a in e.findall("a:author", ns)],
            int(e.findtext("a:published", "0000", ns)[:4]),
            "arXiv",
            url=f"https://arxiv.org/abs/{arxiv_id}",
            arxiv=arxiv_id,
            abstract=" ".join(e.findtext("a:summary", "", ns).split())[:1200],
        ))
    return out
