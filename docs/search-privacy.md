# Built-in web search privacy

What leaves your device when Thuki searches the web, who receives it, and how you control that. For the technical design of the pipeline, see [built-in-web-search.md](./built-in-web-search.md).

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

Until acknowledged, Thuki shows a short v0.16 version announcement on the ask bar (below the input row) as soon as the window opens, whether Auto search is on or off. Tapping **Acknowledge** dismisses it forever (`search_notice_acknowledged` in config). **Turn off/on in Settings** opens Behavior with Auto search highlighted; it does not flip the toggle by itself.

## Hosting note

The public blog post is live at `https://thuki.app/blog/thuki-built-in-web-search`. The in-app learn CTA uses `AUTO_SEARCH_PUBLIC_BLOG_POST_URL` in `src/config/versionAnnouncements.ts`.
