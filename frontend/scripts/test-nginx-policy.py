#!/usr/bin/env python3
"""Adversarial deployment-input checks; HTTP/browser tests live in e2e-production."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("policy", Path(__file__).with_name("render-nginx.py"))
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class PolicyInputs(unittest.TestCase):
    def test_origin_injection_and_ambiguous_urls_rejected(self):
        invalid = ["", "https://good.example; script-src *", "https://good.example\nadd_header X p;",
                   'https://good.example"', "https://good.example/path", "https://good.example?x=1",
                   "https://good.example#x", "https://user:password@good.example", "javascript:alert(1)",
                   "https://good.example\\@evil.example", "https://good.example:65536", "https://-bad.example",
                   "https://good.example:abc", "https://good.example:", "https://*.example", "https://good.example/$host"]
        for value in invalid:
            with self.subTest(value=value), self.assertRaises(ValueError):
                policy.origin(value, "test")

    def test_origin_canonicalization_and_ipv6(self):
        for source, expected in [("https://AUTH.example:443/", "https://auth.example"),
                                 ("http://[::1]:18089", "http://[::1]:18089"),
                                 ("https://auth.example:8443", "https://auth.example:8443")]:
            self.assertEqual(policy.origin(source, "test"), expected)

    def test_each_deployment_field_is_validated(self):
        for field in ["BACKEND_URL", "NYXID_PUBLIC_ORIGIN", "NYXID_BROWSER_CONNECT_ORIGINS"]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.render({field: 'https://example.com"; add_header X injected;'})
        with self.assertRaises(ValueError):
            self.render({"NYXID_BROWSER_CONNECT_ORIGINS": " ".join(["https://example.com"] * 17)})

    def test_split_origin_websockets_and_response_specific_policies(self):
        files = self.render({"NYXID_PUBLIC_ORIGIN": "https://app.example",
                             "NYXID_BROWSER_CONNECT_ORIGINS": "https://api.example:8443"})
        spa, proxy = files["spa-headers.conf"], files["proxy-headers.conf"]
        self.assertIn("wss://api.example:8443", spa)
        self.assertIn("https://api.example:8443", spa)
        self.assertIn("microphone=(self)", spa)
        self.assertNotIn("'unsafe-inline'", spa.split("script-src ", 1)[1].split(";", 1)[0])
        self.assertNotIn("unsafe-eval", spa)
        self.assertIn("$nyxid_proxy_csp", proxy)
        self.assertNotIn("script-src", proxy)
        self.assertIn('Strict-Transport-Security "max-age=31536000"', spa)
        self.assertNotIn("includeSubDomains", spa)
        self.assertNotIn("preload", spa)
        self.assertIn('Strict-Transport-Security ""', self.render({})["spa-headers.conf"])

    def test_default_connect_sources_exclude_browser_analytics_vendors(self):
        spa = self.render({})["spa-headers.conf"]
        connect = spa.split("connect-src ", 1)[1].split('"', 1)[0].split()
        self.assertEqual(set(connect), {
            "'self'", "http://localhost:8080", "ws://localhost:8080",
            "https://api.telegram.org", "https://aevatar-console-backend-api.aevatar.ai",
            "https://www.facebook.com", "https://connect.facebook.net", "https://graph.facebook.com",
            "https://assets.unicorn.studio", "https://storage.googleapis.com/unicornstudio-production/",
            "https://oauth.telegram.org",
        })

    @staticmethod
    def render(env):
        template = Path(__file__).parents[1].joinpath("nginx.conf.template").read_text()
        return policy.render(env, template, Path("/tmp/nyxid-policy"), Path("/tmp/dist"), Path("/tmp/legal-cache.conf"))


if __name__ == "__main__":
    unittest.main()
