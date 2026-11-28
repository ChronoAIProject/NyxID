#!/usr/bin/env python3
"""Replay browser and mobile login against two local replicas sharing a disposable test DB."""
import argparse
from hashlib import sha256
from http.client import HTTPConnection
from http.cookies import SimpleCookie
from urllib.parse import parse_qs, urlencode, urlsplit

MOBILE_RETURN = "nyxid://auth/social/callback"


def request(port, method, path, data=None, cookies=None):
    connection = HTTPConnection("127.0.0.1", port, timeout=10)
    try:
        headers = {"Content-Type": "application/x-www-form-urlencoded"} if data else {}
        if cookies:
            headers["Cookie"] = cookies
        connection.request(method, path, urlencode(data) if data else None, headers)
        response = connection.getresponse()
        assert response.status == 302, (method, path.split("?")[0], response.status)
        return response.getheader("Location"), response.headers.get_all("Set-Cookie") or []
    finally:
        connection.close()


def start(port, provider, mobile):
    path = f"/api/v1/auth/social/{provider}"
    if mobile:
        path += "?" + urlencode({"client": "mobile", "redirect_uri": MOBILE_RETURN})
    location, set_cookies = request(port, "GET", path)
    url = urlsplit(location)
    params = parse_qs(url.query)
    state = params.pop("state")[0]
    nonce = params.pop("nonce", [None])[0]
    apple = provider == "apple"
    assert bool(nonce) == apple
    assert state.startswith("mobile1_") == mobile
    # Several cookies have the same name and different SameSite attributes.
    # Keep their wire order when comparing server versions.
    normalized = []
    active_cookies = []
    for header in set_cookies:
        cookie = SimpleCookie()
        cookie.load(header)
        for name, value in cookie.items():
            if value.value and value["max-age"] != "0":
                active_cookies.append(f"{name}={value.coded_value}")
            if name == "nyx_social_state":
                assert value["httponly"] and value["samesite"] == ("None" if apple else "Lax")
                if apple:
                    assert value["secure"]
                digest = sha256(state.encode()).hexdigest()
                assert value.value == ("mobile1_" if mobile else "") + digest
                normalized.append((name, value.OutputString().replace(value.value, "<digest>", 1)))
            elif name == "nyx_social_nonce":
                assert nonce and value["secure"] and value["samesite"] == "None"
                assert value.value == sha256(nonce.encode()).hexdigest()
                normalized.append((name, value.OutputString().replace(value.value, "<digest>", 1)))
            else:
                normalized.append((name, value.OutputString()))
    assert any(name == "nyx_social_state" for name, _ in normalized)
    assert any(name == "nyx_social_nonce" for name, _ in normalized) == apple
    return state, (url.scheme, url.netloc, url.path, params, normalized), "; ".join(active_cookies)


def cancel(port, provider, state, cookies=None):
    path = f"/api/v1/auth/social/{provider}/callback"
    data = {"state": state, "error": "access_denied"}
    if provider == "apple":
        return request(port, "POST", path, data, cookies=cookies)
    return request(port, "GET", path + "?" + urlencode(data), cookies=cookies)


def compare_browser(old, new, provider):
    attempts = [start(port, provider, False) for port in (old, new)]
    assert attempts[0][1] == attempts[1][1]
    responses = [
        cancel(reader, provider, state, cookies=cookies)
        for (state, _, cookies), reader in zip(attempts, (new, old))
    ]
    assert responses[0] == responses[1], f"{provider} browser callback differs"
    location, headers = responses[0]
    url = urlsplit(location)
    assert url.scheme in ("http", "https") and url.path.endswith("/login")
    assert parse_qs(url.query).get("error") == ["social_auth_denied"]
    cleared = set()
    for header in headers:
        cookie = SimpleCookie()
        cookie.load(header)
        assert len(cookie) == 1
        for name, value in cookie.items():
            assert value.value == "" and value["max-age"] == "0"
            cleared.add(name)
    assert {"nyx_social_state", "nyx_social_nonce"} <= cleared


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline_port", type=int)
    parser.add_argument("candidate_port", type=int)
    args = parser.parse_args()
    for provider in ("google", "github", "apple"):
        old, new = args.baseline_port, args.candidate_port
        compare_browser(old, new, provider)
        attempts = [start(port, provider, True) for port in (old, new)]
        assert attempts[0][1] == attempts[1][1]
        for (state, _, _), reader, writer in zip(attempts, (new, old), (old, new)):
            redirect = urlsplit(cancel(reader, provider, state)[0])
            assert redirect._replace(query="").geturl() == MOBILE_RETURN
            assert parse_qs(redirect.query)["error"] == ["social_auth_denied"]
            replay = urlsplit(cancel(writer, provider, state)[0])
            assert parse_qs(replay.query)["error"] == ["social_auth_csrf"]
        print(f"{provider}: matching authorize redirects and cookies, cancellation and one-time state consumption in both directions")


if __name__ == "__main__":
    main()
