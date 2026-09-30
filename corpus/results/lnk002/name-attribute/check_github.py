import hashlib
import json
import pathlib
import re
import urllib.request
from datetime import datetime, timezone

from playwright.sync_api import sync_playwright

import argparse

parser = argparse.ArgumentParser(description='Recheck pinned GitHub heading name navigation with TLS verification.')
parser.add_argument('--output', type=pathlib.Path, required=True)
OUT = parser.parse_args().output
OUT.mkdir(parents=True, exist_ok=False)
COMMIT = '3fce3b5bb0236da2df6d99672afb8a719642eca7'
RAW = f'https://raw.githubusercontent.com/BurntSushi/ripgrep/{COMMIT}/FAQ.md'
PAGE = f'https://github.com/BurntSushi/ripgrep/blob/{COMMIT}/FAQ.md'


def download(url, data=None):
    headers = {'User-Agent': 'seiso-issue-11-stage-1'}
    if data is not None:
        headers['Content-Type'] = 'application/json'
    request = urllib.request.Request(url, data=data, headers=headers)
    with urllib.request.urlopen(request, timeout=25) as response:
        return response.read()


source = download(RAW)
heading = re.search(rb'<h3 name="config">.*?</h3>', source, re.S).group()
(OUT / 'config-heading.txt').write_bytes(heading)
request_bytes = pathlib.Path(__file__).with_name('markdown-request.json').read_bytes()
request = json.loads(request_bytes)
for block in re.findall(rb'<h3 name="[^"]+">.*?</h3>', source, re.S):
    if block.decode() not in request['text']:
        raise ValueError('Markdown API request omitted an original heading block')
(OUT / 'markdown-request.json').write_bytes(request_bytes)
rendered = download('https://api.github.com/markdown', request_bytes)
(OUT / 'markdown-api.html').write_bytes(rendered)
metadata = {
    'checked_at': datetime.now(timezone.utc).isoformat(),
    'commit': COMMIT, 'source_url': RAW,
    'source_sha256': hashlib.sha256(source).hexdigest(),
    'heading_sha256': hashlib.sha256(heading).hexdigest(),
    'markdown_api_sha256': hashlib.sha256(rendered).hexdigest(),
    'markdown_request_sha256': hashlib.sha256(request_bytes).hexdigest(),
    'certificate_verification': True,
}
cases = {
    'baseline': '',
    'name-config': '#config',
    'ordinary-slug': '#does-ripgrep-support-configuration-files',
    'missing': '#seiso-fragment-control-that-does-not-exist',
}
with sync_playwright() as p:
    browser = p.chromium.launch(executable_path='/usr/bin/chromium',
                                headless=True, args=['--no-sandbox'])
    metadata['browser_version'] = browser.version
    metadata['viewport'] = {'width': 1280, 'height': 800}
    results = []
    for name, fragment in cases.items():
        page = browser.new_page(viewport=metadata['viewport'])
        assets = []
        failures = []
        page.on('response', lambda r: assets.append({'url': r.url, 'status': r.status})
                if 'github.githubassets.com' in r.url else None)
        page.on('requestfailed', lambda r: failures.append({'url': r.url, 'failure': r.failure}))
        response = page.goto(PAGE + fragment, wait_until='domcontentloaded', timeout=25000)
        page.wait_for_selector('article.markdown-body h3[name="user-content-config"]')
        page.wait_for_function('() => document.styleSheets.length > 10')
        page.wait_for_timeout(3500)
        measurement = page.evaluate('''() => {
            const heading = document.querySelector('article.markdown-body h3[name="user-content-config"]');
            const ordinary = document.getElementById('user-content-does-ripgrep-support-configuration-files');
            const fragment = decodeURIComponent(location.hash.slice(1));
            return {hash: location.hash, scroll_y: scrollY,
                    heading_top: heading.getBoundingClientRect().top,
                    heading_document_y: heading.getBoundingClientRect().top + scrollY,
                    heading_html: heading.outerHTML,
                    ordinary_top: ordinary.getBoundingClientRect().top,
                    matching_id: document.getElementById(fragment)?.outerHTML ?? null,
                    matching_name_elements: [...document.getElementsByName(fragment)].map(e => e.outerHTML),
                    prefixed_name_elements: [...document.getElementsByName('user-content-' + fragment)].map(e => e.outerHTML),
                    stylesheet_count: document.styleSheets.length,
                    rendered_font: getComputedStyle(heading).fontFamily};
        }''')
        result = {'case': name, 'url': page.url, 'status': response.status,
                  'measurement': measurement, 'assets': assets, 'request_failures': failures}
        results.append(result)
        metadata['cases'] = results
        (OUT / 'browser.json').write_text(json.dumps(metadata, indent=2) + '\n')
        page.screenshot(path=str(OUT / (name + '.png')))
        print(json.dumps({k: v for k, v in result.items() if k != 'assets'}), flush=True)
        page.close()
    browser.close()
metadata['cases'] = results
(OUT / 'browser.json').write_text(json.dumps(metadata, indent=2) + '\n')
print('markdown_api:', rendered.decode(), flush=True)
