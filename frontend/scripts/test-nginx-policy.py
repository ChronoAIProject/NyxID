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
                             "NYXID_BROWSER_CONNECT_ORIGINS": "https://api.example:8443 https://analytics.example"})
        spa, proxy = files["spa-headers.conf"], files["proxy-headers.conf"]
        self.assertIn("wss://api.example:8443", spa)
        self.assertIn("https://analytics.example", spa)
        self.assertIn("microphone=(self)", spa)
        self.assertNotIn("'unsafe-inline'", spa.split("script-src ", 1)[1].split(";", 1)[0])
        self.assertNotIn("unsafe-eval", spa)
        self.assertIn("$nyxid_proxy_csp", proxy)
        self.assertNotIn("script-src", proxy)
        self.assertIn('Strict-Transport-Security "max-age=31536000"', spa)
        self.assertNotIn("includeSubDomains", spa)
        self.assertNotIn("preload", spa)
        self.assertIn('Strict-Transport-Security ""', self.render({})["spa-headers.conf"])

    def test_all_backend_proxy_locations_clear_unverified_cloudflare_fields(self):
        files = self.render({})
        config, request = files["default.conf"], files["proxy-request-headers.conf"]
        # Every proxy, including the nested reply, includes the same complete
        # request hygiene. Local proxy_set_header disables nginx inheritance.
        locations = []
        for line in config.splitlines():
            if line.strip().startswith("location "):
                locations.append([])
            if locations:
                locations[-1].append(line)
        proxies = ["\n".join(lines) for lines in locations
                   if any("proxy_pass " in line for line in lines)]
        self.assertEqual(len(proxies), 7)
        for location in proxies:
            self.assertIn(f'include {policy.path_literal(Path("/tmp/nyxid-policy/proxy-request-headers.conf"))};', location)
        self.assertNotIn("proxy_set_header CF-", config)
        for field in ["CF-Connecting-IP", "CF-IPCountry", "CF-IPCity", "CF-Region",
                      "CF-IPContinent", "CF-Timezone"]:
            self.assertEqual(request.count(f'proxy_set_header {field} "";'), 1)
        self.assertIn('proxy_set_header Forwarded "";', request)
        self.assertIn('proxy_set_header X-Real-IP $remote_addr;', request)
        self.assertIn('proxy_set_header X-Forwarded-For $remote_addr;', request)
        self.assertNotIn("$proxy_add_x_forwarded_for", request)

    def test_outer_ingress_accepts_only_bounded_explicit_ips_and_cidrs(self):
        for value in ["*", "0.0.0.0/0", "::/0", "::ffff:0.0.0.0/96", "0.0.0.0", "::", "unix:", "localhost",
                      "127.0.0.1:8080", "127.0.0.1; include /tmp/inject;", "127.0.0.1\n",
                      "127.0.0.1,", ",127.0.0.1", "fe80::1%en0", "deadbeef", ",".join(["127.0.0.1"] * 65)]:
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "NYXID_TRUSTED_INGRESS_IPS"):
                self.render({"NYXID_TRUSTED_INGRESS_IPS": value})
        request = self.render({"NYXID_TRUSTED_INGRESS_IPS": "::ffff:172.30.0.1, 192.168.1.42/24,2001:db8::1/128"})[
            "proxy-request-headers.conf"]
        for network in ["172.30.0.1/32", "192.168.1.0/24", "2001:db8::1/128"]:
            self.assertIn(f"set_real_ip_from {network};", request)
        self.assertIn("real_ip_header X-Forwarded-For;", request)
        self.assertIn("real_ip_recursive on;", request)
        self.assertNotIn("set_real_ip_from", self.render({})["proxy-request-headers.conf"])

    @staticmethod
    def render(env):
        template = Path(__file__).parents[1].joinpath("nginx.conf.template").read_text()
        return policy.render(env, template, Path("/tmp/nyxid-policy"), Path("/tmp/dist"), Path("/tmp/legal-cache.conf"))


if __name__ == "__main__":
    unittest.main()
