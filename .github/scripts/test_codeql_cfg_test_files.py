import tempfile
import unittest
import shutil
import subprocess
from pathlib import Path

from codeql_cfg_test_files import analyze_tree, exclusion_list


def crate(root: Path, files: dict[str, str], manifest: str | None = None) -> None:
    text = manifest or '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n'
    (root / "Cargo.toml").write_text(text, encoding="utf-8")
    for rel, body in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")


class CfgTestFileTests(unittest.TestCase):
    def excluded(self, files: dict[str, str], manifest: str | None = None) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            crate(root, files, manifest)
            return exclusion_list(root)

    def test_filename_is_not_the_rule(self):
        found = self.excluded({
            "src/lib.rs": "mod tests;\n#[cfg(test)]\nmod prod;\n",
            "src/tests.rs": "pub fn shipped() {}\n",
            "src/prod.rs": "fn only_tests() {}\n",
        })
        self.assertEqual(found, ["src/prod.rs"])

    def test_cfg_forms(self):
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "#[cfg(all(test, feature = \"x\"))]\nmod gated;\n",
                "src/gated.rs": "fn t() {}\n",
            }),
            ["src/gated.rs"],
        )
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "#[cfg(any(test, feature = \"test-support\"))]\nmod support;\n",
                "src/support.rs": "pub fn helper() {}\n",
            }),
            [],
        )
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "#[cfg(all(test, unix))]\nmod unix_tests;\n",
                "src/unix_tests.rs": "fn t() {}\n",
            }),
            ["src/unix_tests.rs"],
        )

    def test_inline_module_excludes_no_file(self):
        self.assertEqual(
            self.excluded({"src/lib.rs": "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n"}),
            [],
        )

    def test_include_and_path(self):
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "include!(\"runtime.rs\");\n#[cfg(test)]\nmod tests {\n    include!(\"only_tests.rs\");\n}\n",
                "src/runtime.rs": "pub fn prod() {}\n",
                "src/only_tests.rs": "fn t() {}\n",
            }),
            ["src/only_tests.rs"],
        )
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "#[cfg(test)]\n#[path = \"real_tests.rs\"]\nmod tests;\n",
                "src/real_tests.rs": "fn t() {}\n",
                "src/tests.rs": "pub fn shipped() {}\n",
            }),
            ["src/real_tests.rs"],
        )

    def test_cfg_attr_path_splits_production_and_test_files(self):
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "#[cfg_attr(test, path = \"t.rs\")]\nmod foo;\n",
                "src/foo.rs": "pub fn prod() {}\n",
                "src/t.rs": "fn t() {}\n",
            }),
            ["src/t.rs"],
        )

    def test_unresolvable_production_include_poisons_the_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            crate(root, {
                "src/lib.rs": "include!(concat!(\"a\", \".rs\"));\n#[cfg(test)]\nmod secret;\n",
                "src/secret.rs": "fn t() {}\n",
            })
            result = analyze_tree(root)
            self.assertTrue(result.poisoned)
            self.assertEqual(result.excluded(), [])

    def test_unresolvable_test_include_does_not_poison(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            crate(root, {
                "src/lib.rs": "#[cfg(test)]\nmod tests {\n    include!(concat!(\"a\", \".rs\"));\n}\n#[cfg(test)]\nmod secret;\n",
                "src/secret.rs": "fn t() {}\n",
            })
            result = analyze_tree(root)
            self.assertEqual(result.poisoned, {})
            self.assertEqual(result.excluded(), ["src/secret.rs"])

    def test_unresolved_production_edges_can_cross_package_boundaries(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in ("a", "b"):
                (root / name).mkdir()
            crate(root / "a", {"src/lib.rs": 'include!(concat!("../../b/src/", "shared.rs"));'})
            crate(root / "b", {"src/lib.rs": '#[cfg(test)] mod shared;',
                                "src/shared.rs": 'pub fn shipped_from_a() {}'})
            self.assertEqual(exclusion_list(root), [])

    def test_macro_include_does_not_guess_invocation_module_directory(self):
        self.assertEqual(self.excluded({
            "src/lib.rs": 'macro_rules! load { () => { include!("shared.rs"); }; } '
                          '#[cfg(test)] mod shared;',
            "src/shared.rs": 'pub fn run() {}',
        }), [])

    def test_literal_paths_cannot_become_codeql_wildcards(self):
        self.assertEqual(self.excluded({
            "src/lib.rs": '#[cfg(test)] #[path="wild*.rs"] mod tests;',
            "src/wild*.rs": 'fn test_only() {}',
        }), [])

    def test_integration_root_stays_and_inner_cfg_test_file_goes(self):
        # tests/hidden.rs is its own integration-test crate root, so it stays.
        # The nested file is reached only from a cfg(test) module.
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "pub fn prod() {}\n",
                "tests/it.rs": "#[cfg(test)]\nmod hidden {\n    mod nested;\n}\nfn main() {}\n",
                "tests/hidden.rs": "fn also_a_target() {}\n",
                "tests/hidden/nested.rs": "fn t() {}\n",
            }),
            ["tests/hidden/nested.rs"],
        )

    def test_module_file_layout(self):
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "mod parent;\n",
                "src/parent.rs": "#[cfg(test)]\nmod child;\n",
                "src/parent/child.rs": "fn t() {}\n",
                "src/child.rs": "pub fn decoy() {}\n",
            }),
            ["src/parent/child.rs"],
        )
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "#[path = \"elsewhere.rs\"]\nmod renamed;\n",
                "src/elsewhere.rs": "#[cfg(test)]\nmod sibling;\n",
                "src/sibling.rs": "fn t() {}\n",
                "src/elsewhere/sibling.rs": "pub fn decoy() {}\n",
            }),
            ["src/sibling.rs"],
        )

    def test_inner_cfg_test_attribute(self):
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "mod inner;\n",
                "src/inner.rs": "#![cfg(test)]\nfn t() {}\n",
            }),
            ["src/inner.rs"],
        )

    def test_path_inside_inline_module_uses_the_module_directory(self):
        self.assertEqual(
            self.excluded({
                "src/lib.rs": "mod outer {\n    #[cfg(test)]\n    #[path = \"x.rs\"]\n    mod inner;\n}\n",
                "src/outer/x.rs": "fn t() {}\n",
            }),
            ["src/outer/x.rs"],
        )

    def test_list_is_deterministic(self):
        files = {
            "src/lib.rs": "#[cfg(test)]\nmod b;\n#[cfg(test)]\nmod a;\n",
            "src/a.rs": "fn t() {}\n",
            "src/b.rs": "fn t() {}\n",
        }
        self.assertEqual(self.excluded(files), self.excluded(files))
        self.assertEqual(self.excluded(files), ["src/a.rs", "src/b.rs"])

    def test_raw_string_backslashes_are_literal(self):
        source = 'const R: &str = r"(?i)Bearer\\s+\\S+";\n#[cfg(test)]\nmod secret;\n'
        self.assertEqual(
            self.excluded({"src/lib.rs": source, "src/secret.rs": "fn t() {}\n"}),
            ["src/secret.rs"],
        )

    def test_string_continuation_hex_and_unicode_do_not_poison(self):
        source = (
            'const A: &str = "foo\\\n'
            '         bar\\x01\\u{2E}rs";\n'
            'const B: &[u8] = b"\\xFF";\n'
            "const C: char = '\\x01';\n"
            "#[cfg(test)]\n"
            "mod secret;\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            crate(root, {"src/lib.rs": source, "src/secret.rs": "fn t() {}\n"})
            result = analyze_tree(root)
            self.assertEqual(result.poisoned, {})
            self.assertEqual(result.excluded(), ["src/secret.rs"])

    def test_comments_and_strings_do_not_declare_modules(self):
        source = '''
            // mod commented;
            /* mod blocked; */
            const TEXT: &str = "mod string;";
            fn keep() {
                let life = 'static;
                let ch = 'x';
                let raw = r#"mod raw;"#;
            }
            #[cfg(test)]
            mod real;
        '''
        self.assertEqual(
            self.excluded({"src/lib.rs": source, "src/real.rs": "fn t() {}\n"}),
            ["src/real.rs"],
        )

    def test_automatic_targets_alongside_explicit_targets_remain_production(self):
        manifest = ('[package]\nname="demo"\nversion="0.1.0"\nedition="2021"\n'
                    '[[bin]]\nname="manual"\npath="src/manual.rs"\n')
        self.assertEqual(self.excluded({
            "src/lib.rs": '#[cfg(test)] mod shared;',
            "src/shared.rs": 'pub fn run() {}',
            "src/manual.rs": 'fn main() {}',
            "src/bin/auto.rs": '#[path="../shared.rs"] mod shared; fn main() { shared::run(); }',
        }, manifest), [])

    def test_const_function_does_not_absorb_following_module_attributes(self):
        self.assertEqual(self.excluded({
            "src/lib.rs": 'pub const fn value() -> u8 { 1 } #[cfg(test)] mod shared;',
            "src/shared.rs": 'fn test_only() {}',
        }), ["src/shared.rs"])

    def test_macro_with_only_inline_modules_does_not_load_other_files(self):
        self.assertEqual(self.excluded({
            "src/lib.rs": ('macro_rules! make { ($name:ident) => { '
                           'pub mod $name { pub fn value() {} } }; } '
                           'make!(inline); #[cfg(test)] mod shared;'),
            "src/shared.rs": 'fn test_only() {}',
        }), ["src/shared.rs"])

    def test_production_references_hidden_in_comments_blocks_macros_and_paths(self):
        cases = [
            ('#[cfg(test)] #[path="shared.rs"] mod tests; const fn helper() {} '
             'mod shared; pub fn run() { helper(); shared::run(); }',
             {"src/shared.rs": 'pub fn run() {}'}),
            ('/* outer /* inner */ #[cfg(test)] */ mod shared; '
             'pub fn run() { shared::run(); }', {"src/shared.rs": 'pub fn run() {}'}),
            ('#[cfg(test)] #[path="shared.rs"] mod tests; '
             'pub fn run() { #[path="shared.rs"] mod nested; nested::run(); }',
             {"src/shared.rs": 'pub fn run() {}'}),
            ('macro_rules! load { () => { #[path="shared.rs"] mod nested; }; } load!(); '
             '#[cfg(test)] #[path="shared.rs"] mod tests; pub fn run() { nested::run(); }',
             {"src/shared.rs": 'pub fn run() {}'}),
            ('#[cfg_attr(feature="alternate",path="alternate")] mod parent { pub mod child; } '
             '#[cfg(test)] #[path="parent/child.rs"] mod tests; '
             'pub fn run() { parent::child::run(); }',
             {"src/parent/child.rs": 'pub fn run() {}',
              "src/alternate/child.rs": 'pub fn run() {}'}),
        ]
        for source, files in cases:
            with self.subTest(source=source), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                crate(root, {"src/lib.rs": source, **files})
                # These are valid production Rust, not merely lexer strings.
                if shutil.which("rustc"):
                    result = subprocess.run(
                        ["rustc", "--edition=2021", "--crate-type=lib", "src/lib.rs"],
                        cwd=root, text=True, capture_output=True, timeout=60,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(exclusion_list(root), [])


class WorkspaceTests(unittest.TestCase):
    def test_this_repo_keeps_production_files(self):
        root = Path(__file__).resolve().parents[2]
        result = analyze_tree(root)
        excluded = set(result.excluded())
        self.assertNotIn("backend/src/main.rs", excluded)
        self.assertNotIn("cli/src/main.rs", excluded)
        self.assertNotIn("cli/src/node/machine/runtime.rs", excluded)
        self.assertNotIn("service-adapters/src/test_support.rs", excluded)
        self.assertNotIn("cli/src/node/machine/native_desktop_linux.rs", excluded)
        self.assertIn("machine-updater/src/kubernetes_credentials_tests.rs", excluded)
        self.assertIn("backend/src/handlers/proxy_concurrency_tests.rs", excluded)
        for path in excluded:
            self.assertTrue(path.endswith(".rs"), path)
            self.assertFalse(path.startswith("target/"), path)
        self.assertEqual(excluded, set(exclusion_list(root)))


if __name__ == "__main__":
    unittest.main()
