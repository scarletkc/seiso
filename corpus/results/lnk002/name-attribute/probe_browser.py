"""Record live GitHub fragment navigation; requires Playwright and Chromium.

Run from the repository root, passing a NEW output file:
python corpus/results/lnk002/name-attribute/probe_browser.py --output /tmp/browser.json
Do not interpret a negative result unless the ordinary-slug control succeeds
and the GitHub scripts loaded successfully.
"""

import argparse
import datetime
import json
from pathlib import Path

from playwright.sync_api import sync_playwright


URL = "https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/FAQ.md"
FRAGMENTS = [
    "config",
    "does-ripgrep-support-configuration-files",
    "seiso-name-attribute-missing-control",
    "user-content-does-ripgrep-support-configuration-files",
]
MEASURE = """() => {
    const h = document.querySelector('h3[name="user-content-config"]');
    const a = document.getElementById('user-content-does-ripgrep-support-configuration-files');
    return {
        url: location.href, scroll_y: scrollY,
        heading_rect: h?.getBoundingClientRect().toJSON(),
        heading_document_y: h?.getBoundingClientRect().top + scrollY,
        heading_html: h?.outerHTML,
        ordinary_anchor_rect: a?.getBoundingClientRect().toJSON(),
        ordinary_anchor_html: a?.outerHTML,
        ready_state: document.readyState,
        scrolling_element: document.scrollingElement?.tagName,
        body_height: document.body.scrollHeight
    };
}"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--chromium", default="/usr/bin/chromium")
    parser.add_argument("--ignore-https-errors", action="store_true",
                        help="Needed only for the managed environment's HTTPS proxy")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Preserve previous evidence: output already exists")
    checks = []
    with sync_playwright() as pw:
        browser = pw.chromium.launch(executable_path=args.chromium, headless=True,
                                     args=["--no-sandbox"])
        for fragment in FRAGMENTS:
            context = browser.new_context(viewport={"width": 1280, "height": 900},
                                          ignore_https_errors=args.ignore_https_errors)
            page = context.new_page()
            failures = []
            page.on("requestfailed", lambda request: failures.append({
                "url": request.url, "error": request.failure}))
            response = page.goto(URL + "#" + fragment, wait_until="domcontentloaded",
                                 timeout=60000)
            page.wait_for_timeout(8000)
            check = page.evaluate(MEASURE)
            check.update(fragment=fragment, http_status=response.status,
                         request_failures=failures)
            checks.append(check)
            print(fragment, check["scroll_y"], len(failures), flush=True)
            context.close()
        evidence = {
            "observed_at": datetime.datetime.now(datetime.UTC).isoformat(),
            "browser": browser.version, "viewport": {"width": 1280, "height": 900},
            "wait_after_domcontentloaded_ms": 8000,
            "ignore_https_errors": args.ignore_https_errors, "checks": checks,
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as output:
            output.write(json.dumps(evidence, indent=2) + "\n")
        browser.close()


if __name__ == "__main__":
    main()
