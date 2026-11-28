"""The publish matrix exposes multiline upload JSON only on frontend/amd64."""
import re
import unittest
from pathlib import Path

from workflow_test_helpers import USES_WITH_TAG, load_workflow_with_action_tags


def evaluate(expression, context):
    """Evaluate only the closed, side-effect-free GitHub subset used here."""
    if not expression.startswith('${{') or not expression.endswith('}}'):
        raise ValueError('Expected a GitHub expression')
    source = expression[3:-2].strip()
    token = re.compile(r"\s*(==|!=|&&|\|\||'(?:[^']|'')*'|(?:matrix|secrets)\.[A-Za-z_][A-Za-z_0-9]*)")
    tokens = []
    position = 0
    while position < len(source):
        match = token.match(source, position)
        if not match:
            raise ValueError(f'Unsupported expression at {position}')
        tokens.append(match.group(1))
        position = match.end()
    cursor = 0

    def atom():
        nonlocal cursor
        if cursor == len(tokens):
            raise ValueError('Missing operand')
        value = tokens[cursor]
        cursor += 1
        if value.startswith("'"):
            return value[1:-1].replace("''", "'")
        if '.' not in value:
            raise ValueError('Expected literal or context lookup')
        namespace, name = value.split('.')
        # Unset secrets resolve to the empty string in GitHub expressions.
        return context[namespace].get(name, '')

    def equality():
        nonlocal cursor
        left = atom()
        while cursor < len(tokens) and tokens[cursor] in ('==', '!='):
            operator = tokens[cursor]
            cursor += 1
            right = atom()
            # All comparable operands in this subset are strings; GitHub's
            # string comparisons ignore case. Equality results are booleans.
            equal = (left.casefold() == right.casefold()
                     if isinstance(left, str) and isinstance(right, str)
                     else left == right)
            left = equal if operator == '==' else not equal
        return left

    def conjunction():
        nonlocal cursor
        left = equality()
        while cursor < len(tokens) and tokens[cursor] == '&&':
            cursor += 1
            right = equality()
            left = right if left else left
        return left

    value = conjunction()
    while cursor < len(tokens) and tokens[cursor] == '||':
        cursor += 1
        right = conjunction()
        value = value if value else right
    if cursor != len(tokens):
        raise ValueError('Unexpected trailing tokens')
    return value


class CrashArtifactSecretsTests(unittest.TestCase):
    def test_expression_subset_preserves_values_and_operator_precedence(self):
        context = {'matrix': {'component': 'frontend'}, 'secrets': {}}
        self.assertEqual(evaluate("${{ '' && 'no' || 'fallback' }}", context), 'fallback')
        self.assertEqual(evaluate("${{ 'left' || '' && 'no' }}", context), 'left')
        self.assertEqual(evaluate("${{ matrix.component == 'FRONTEND' && 'value' || '' }}", context), 'value')
        self.assertEqual(evaluate("${{ secrets.UNSET != '' && 'no' || '' }}", context), '')
        self.assertEqual(evaluate("${{ 'it''s literal' }}", context), "it's literal")
        with self.assertRaises(ValueError):
            evaluate("${{ arbitrary() }}", context)

    def test_upload_secret_mapping_is_empty_except_configured_frontend_amd64(self):
        workflow = load_workflow_with_action_tags('.github/workflows/publish-images.yml')
        step = next(s for s in workflow['jobs']['build']['steps'] if s.get('id') == 'build')
        self.assertEqual(step.get(USES_WITH_TAG), 'docker/build-push-action@v7')
        self.assertNotIn('secrets', step['with'])
        mapping = step['with']['secret-envs']
        self.assertEqual(mapping, "${{ matrix.component == 'frontend' && matrix.platform == 'linux/amd64' && secrets.CRASH_ARTIFACTS_SENTRY != '' && 'crash_artifacts_sentry=CRASH_ARTIFACTS_SENTRY' || '' }}")
        value = step['env']['CRASH_ARTIFACTS_SENTRY']
        self.assertEqual(value, "${{ matrix.component == 'frontend' && matrix.platform == 'linux/amd64' && secrets.CRASH_ARTIFACTS_SENTRY || '' }}")
        matrix = workflow['jobs']['build']['strategy']['matrix']
        legs = [dict(component=c, platform=p) for c in matrix['component'] for p in matrix['platform']]
        self.assertEqual(len(legs), 10)
        for leg in legs:
            for secret in [None, '', '{"token":"sentinel"}', '{\n  "token": "sentinel"\n}']:
                with self.subTest(leg=leg, secret=secret):
                    context = {'matrix': leg, 'secrets': {} if secret is None else {'CRASH_ARTIFACTS_SENTRY': secret}}
                    allowed = leg['component'] == 'frontend' and leg['platform'] == 'linux/amd64'
                    expected = 'crash_artifacts_sentry=CRASH_ARTIFACTS_SENTRY' if allowed and secret else ''
                    self.assertEqual(evaluate(mapping, context), expected)
                    self.assertEqual(evaluate(value, context), (secret or '') if allowed else '')

    def test_docker_upload_secret_is_optional_and_empty_file_is_a_noop(self):
        dockerfile = Path('frontend/Dockerfile').read_text()
        mount = re.search(r'--mount=type=secret,id=crash_artifacts_sentry([^\s]*)', dockerfile)
        self.assertIsNotNone(mount)
        self.assertNotIn('required=true', mount.group(0))
        self.assertIn('if [ -s /run/secrets/crash_artifacts_sentry ]; then', dockerfile)


if __name__ == '__main__':
    unittest.main()
