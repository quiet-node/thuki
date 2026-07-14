# How Thuki web search works

Thuki can look up current information on the web when a question needs fresh facts. This page explains what leaves your device, who receives it, and how you control that.

## Queries leave your device

When search runs, Thuki sends the query **directly from your Mac** to one or more of these services:

1. [DuckDuckGo](https://duckduckgo.com/) (HTML search results)
2. [Mojeek](https://www.mojeek.com/) (search results)
3. [Open-Meteo](https://open-meteo.com/) (weather)
4. [Google News RSS](https://news.google.com/) (headlines)
5. [Wikipedia](https://www.wikipedia.org/) (definitions and stable facts)
6. [ESPN public scoreboard API](https://www.espn.com/) (live scores and schedules; unofficial frontend API)

Which services are contacted depends on the question (for example weather uses Open-Meteo; sports scores use ESPN; general web questions use DuckDuckGo and Mojeek).

## Thuki has no servers for search

Thuki does **not** proxy search through Quiet Node or any Thuki backend. Queries never pass through Thuki-operated servers. The app has no cloud account for search and never sees your queries after they leave the device toward those third-party services.

Answers are assembled on your Mac from the responses those services return.

## Provider privacy policies

Read each service’s own policy for how they handle requests from your network:

| Service | Privacy / policy |
| :------ | :--------------- |
| DuckDuckGo | [https://duckduckgo.com/privacy](https://duckduckgo.com/privacy) |
| Mojeek | [https://www.mojeek.com/privacy](https://www.mojeek.com/privacy) |
| Open-Meteo | [https://open-meteo.com/en/terms](https://open-meteo.com/en/terms) (terms and attribution; contact site for privacy details) |
| Google News / Google | [https://policies.google.com/privacy](https://policies.google.com/privacy) |
| Wikipedia / Wikimedia | [https://foundation.wikimedia.org/wiki/Policy:Privacy_policy](https://foundation.wikimedia.org/wiki/Policy:Privacy_policy) |
| ESPN / Disney | [https://privacy.thewaltdisneycompany.com/en/](https://privacy.thewaltdisneycompany.com/en/) |

Links can change; if one breaks, start from the provider’s homepage and open their Privacy or Terms page.

## User-Agent policy

Thuki identifies itself honestly on **API verticals** (Open-Meteo, Google News RSS, ESPN scoreboard):

```text
Thuki/<version> (+https://thuki.app)
```

Wikipedia keeps a Wikimedia-compliant descriptive User-Agent (product name, homepage, contact).

**Search engines (DuckDuckGo HTML and Mojeek) deliberately use a normal browser User-Agent.** An honest bot-style User-Agent on DuckDuckGo’s `/html` endpoint is blocked immediately. The tradeoff is intentional: browser-like UA for SERP reachability, plus a cooldown that does not hammer a blocked engine so we do not retry through a block in a tight loop.

## How you control search

- **Settings → Behavior → Auto search** (on by default): plain questions may search when live facts are needed.
- Turn **Auto search** off to stay fully local unless you force a search.
- **`/search`** always forces a web look-up for that message, even when Auto search is off.

When Auto search is on and the notice has not been acknowledged, Thuki shows a short elevated panel on the ask bar (above the input row) as soon as the window opens. Tapping **Got it** dismisses it forever (`search_notice_acknowledged` in config). **Turn off in Settings** opens Behavior with Auto search highlighted; it does not flip the toggle by itself.

## Hosting note

This file is the copy draft for a future **blog post** on thuki.app. Title and slug are TBD. In-app **How Auto search works** currently opens `https://thuki.app/blog` (blog index placeholder). When the dedicated post is live, point `SEARCH_DISCLOSURE_URL` in `src/components/SearchTrustNotice.tsx` at that post (see issue #320).
