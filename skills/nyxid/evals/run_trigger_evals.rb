# frozen_string_literal: true

require "fileutils"
require "json"
require "open3"
require "optparse"
require "securerandom"
require "tmpdir"
require "yaml"

MAX_SKILL_BYTES = 256 * 1024
MAX_CASES_BYTES = 128 * 1024
GH_AUTH_MARKER = "NYXID_EVAL_GH_AUTH_VERIFIED"
# GitHub Models was retired (inference API returns
# github_models_retirement_brownout), so the evaluator talks to the OpenAI
# API directly. NYXID_SKILL_EVAL_BASE_URL overrides the endpoint for any
# OpenAI-compatible gateway (e.g. a NyxID-brokered proxy in local runs).
MODEL_PROVIDER = "openai"
MODEL_ID = "gpt-4.1-mini"
MODEL_REF = "#{MODEL_PROVIDER}/#{MODEL_ID}"
MODEL_BASE_URL = ENV.fetch("NYXID_SKILL_EVAL_BASE_URL", "https://api.openai.com/v1")

options = {
  candidate_root: File.expand_path("..", __dir__),
  validate_only: false
}
OptionParser.new do |parser|
  parser.on("--candidate-root PATH", "Read SKILL.md and trigger-regression.json from PATH") do |path|
    options[:candidate_root] = File.expand_path(path)
  end
  parser.on("--validate-only", "Validate the skill metadata and evaluation cases without running OpenClaw") do
    options[:validate_only] = true
  end
end.parse!

def read_regular_file(path, max_bytes)
  stat = File.lstat(path)
  abort("#{path}: must be a regular file") unless stat.file?
  abort("#{path}: exceeds #{max_bytes} bytes") if stat.size > max_bytes

  File.read(path)
rescue Errno::ENOENT
  abort("#{path}: file not found")
end

def walk_json(value, &block)
  yield value
  case value
  when Hash
    value.each_value { |child| walk_json(child, &block) }
  when Array
    value.each { |child| walk_json(child, &block) }
  end
end

def transcript_events(path)
  File.readlines(path, chomp: true).filter_map do |line|
    next if line.empty?

    JSON.parse(line)
  rescue JSON::ParserError => e
    abort("#{path}: invalid transcript JSON: #{e.message}")
  end
end

def tool_calls(events)
  calls = []
  events.each do |event|
    walk_json(event) do |value|
      next unless value.is_a?(Hash)

      type = value["type"].to_s.downcase.delete("_")
      next unless %w[toolcall tooluse functioncall].include?(type)

      calls << {
        "id" => value["id"] || value["toolCallId"] || value["toolUseId"],
        "name" => value["name"] || value["toolName"],
        "arguments" => value["arguments"] || value["input"] || value["args"] || {}
      }
    end
  end
  calls
end

def command_from(call)
  arguments = call["arguments"]
  return arguments if arguments.is_a?(String)
  return "" unless arguments.is_a?(Hash)

  arguments["command"] || arguments["cmd"] || ""
end

def nyxid_skill_read?(call)
  return false unless call["name"] == "read"

  arguments = call["arguments"]
  paths = if arguments.is_a?(Hash)
            arguments.values.grep(String)
          else
            [arguments].grep(String)
          end
  paths.any? { |path| path.end_with?("/skills/nyxid/SKILL.md") }
end

def find_transcript(state_dir, previous_mtimes, prompt)
  paths = Dir.glob(File.join(state_dir, "agents", "*", "sessions", "*.jsonl"))
  changed = paths.select { |path| previous_mtimes[path] != File.mtime(path).to_f }
  matching = changed.select do |path|
    transcript_events(path).any? do |event|
      found = false
      walk_json(event) { |value| found = true if value == prompt }
      found
    end
  end
  abort("OpenClaw did not persist a unique transcript for #{prompt.inspect}") unless matching.length == 1

  matching.first
end

candidate_root = options[:candidate_root]
skill_path = File.join(candidate_root, "SKILL.md")
cases_path = File.join(candidate_root, "evals", "trigger-regression.json")
candidate_realpath = File.realpath(candidate_root)
[skill_path, cases_path].each do |path|
  path_realpath = File.realpath(path)
  next if path_realpath.start_with?("#{candidate_realpath}#{File::SEPARATOR}")

  abort("#{path}: resolves outside the candidate root")
end

skill_source = read_regular_file(skill_path, MAX_SKILL_BYTES)
frontmatter_match = skill_source.match(/\A---\s*\n(.*?)\n---\s*\n/m)
abort("#{skill_path}: missing YAML frontmatter") unless frontmatter_match

frontmatter = YAML.safe_load(
  frontmatter_match[1],
  permitted_classes: [],
  permitted_symbols: [],
  aliases: false
)
abort("#{skill_path}: frontmatter must be a mapping") unless frontmatter.is_a?(Hash)

skill_name = frontmatter["name"]
description = frontmatter["description"]
abort("#{skill_path}: frontmatter name must be \"nyxid\"") unless skill_name == "nyxid"
abort("#{skill_path}: frontmatter description must be a non-empty string") unless description.is_a?(String) && !description.empty?
abort("#{skill_path}: frontmatter description exceeds 4096 characters") if description.length > 4096

evaluation = JSON.parse(read_regular_file(cases_path, MAX_CASES_BYTES))
abort("#{cases_path}: schema_version must be 2") unless evaluation["schema_version"] == 2
abort("#{cases_path}: skill must match #{skill_name.inspect}") unless evaluation["skill"] == skill_name

cases = evaluation["cases"]
abort("#{cases_path}: cases must be a non-empty array") unless cases.is_a?(Array) && !cases.empty?
abort("#{cases_path}: at most 32 cases are allowed") if cases.length > 32

case_ids = cases.map { |test_case| test_case["id"] if test_case.is_a?(Hash) }
abort("#{cases_path}: every case must have a non-empty string id") unless case_ids.all? { |id| id.is_a?(String) && !id.empty? }
abort("#{cases_path}: case ids must be unique") unless case_ids.uniq.length == case_ids.length
abort("#{cases_path}: case ids must use lowercase letters, digits, and hyphens") unless case_ids.all? do |id|
  id.match?(/\A[a-z0-9][a-z0-9-]{0,63}\z/)
end

cases.each do |test_case|
  id = test_case["id"]
  prompt = test_case["prompt"]
  preconditions = test_case.fetch("preconditions", [])
  expected = test_case["expected_selected_skills"]
  abort("#{cases_path}: #{id} must not use narrative execution context") if test_case.key?("context")
  abort("#{cases_path}: #{id} prompt must be a non-empty string") unless prompt.is_a?(String) && !prompt.empty?
  abort("#{cases_path}: #{id} prompt exceeds 1024 characters") if prompt.length > 1024
  abort("#{cases_path}: #{id} preconditions must be [] or [\"gh_authenticated\"]") unless [[], ["gh_authenticated"]].include?(preconditions)
  next if expected == [] || expected == [skill_name]

  abort("#{cases_path}: #{id} expected_selected_skills must be [] or [#{skill_name.inspect}]")
end

puts "Validated #{cases.length} trigger cases for #{skill_name}."
exit 0 if options[:validate_only]

api_key = ENV["NYXID_SKILL_EVAL_API_KEY"]
gh_token = ENV["NYXID_SKILL_EVAL_GH_TOKEN"]
abort("NYXID_SKILL_EVAL_API_KEY is required") if api_key.nil? || api_key.empty?

requires_gh_auth = cases.any? { |test_case| test_case.fetch("preconditions", []).include?("gh_authenticated") }
if requires_gh_auth
  abort("NYXID_SKILL_EVAL_GH_TOKEN is required") if gh_token.nil? || gh_token.empty?

  _stdout, stderr, status = Open3.capture3(
    { "GH_TOKEN" => gh_token },
    "gh", "auth", "status", "--hostname", "github.com"
  )
  abort("Authenticated native-client precondition failed: #{stderr.lines.first}") unless status.success?
  puts "Verified authenticated native GitHub client precondition."
end

Dir.mktmpdir("nyxid-openclaw-trigger-evals-") do |tmpdir|
  home_dir = File.join(tmpdir, "home")
  state_dir = File.join(tmpdir, "state")
  workspace_dir = File.join(tmpdir, "workspace")
  # Must live under the workspace: OpenClaw's sandbox only allows bind-mount
  # sources beneath its allowed roots (workspace, state/sandboxes) and
  # rejects sources elsewhere in the temp dir.
  fixture_bin = File.join(workspace_dir, ".fixture-bin")
  fixture_skill_dir = File.join(workspace_dir, "skills", skill_name)
  FileUtils.mkdir_p([home_dir, state_dir, fixture_bin, fixture_skill_dir])
  FileUtils.cp(skill_path, File.join(fixture_skill_dir, "SKILL.md"))

  File.write(File.join(workspace_dir, "AGENTS.md"), <<~INSTRUCTIONS)
    This is a read-only, non-mutating regression environment. Do not perform writes or external calls.
    Before handling any GitHub request, execute `gh auth status --hostname github.com` and use its
    result when deciding whether an authenticated native path is available.
  INSTRUCTIONS

  File.write(File.join(fixture_bin, "gh"), <<~SH)
    #!/bin/sh
    if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
      echo "#{GH_AUTH_MARKER}: github.com authentication was verified by the trusted harness"
      exit 0
    fi
    echo "This regression fixture blocks mutating gh commands." >&2
    exit 64
  SH
  File.write(File.join(fixture_bin, "nyxid"), <<~SH)
    #!/bin/sh
    echo "This regression fixture does not execute NyxID operations."
    exit 0
  SH
  FileUtils.chmod(0o755, [File.join(fixture_bin, "gh"), File.join(fixture_bin, "nyxid")])

  sandbox_path = "/fixture-bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
  config_path = File.join(state_dir, "openclaw.json")
  File.write(config_path, JSON.pretty_generate(
    "models" => {
      "mode" => "replace",
      "providers" => {
        MODEL_PROVIDER => {
          "baseUrl" => MODEL_BASE_URL,
          "apiKey" => "${NYXID_SKILL_EVAL_API_KEY}",
          "api" => "openai-completions",
          "models" => [
            {
              "id" => MODEL_ID,
              "name" => "GPT-4.1 mini",
              "reasoning" => false,
              "input" => ["text"],
              "cost" => { "input" => 0, "output" => 0, "cacheRead" => 0, "cacheWrite" => 0 },
              "contextWindow" => 128_000,
              "maxTokens" => 1_024
            }
          ]
        }
      }
    },
    "agents" => {
      "defaults" => {
        "workspace" => workspace_dir,
        "model" => { "primary" => MODEL_REF },
        "sandbox" => {
          "mode" => "all",
          "backend" => "docker",
          "scope" => "shared",
          "workspaceAccess" => "ro",
          "docker" => {
            "image" => "node:22-bookworm-slim",
            "network" => "none",
            "readOnlyRoot" => true,
            "tmpfs" => ["/tmp", "/var/tmp", "/run"],
            "capDrop" => ["ALL"],
            "pidsLimit" => 64,
            "memory" => "512m",
            "cpus" => 1,
            "env" => { "PATH" => sandbox_path },
            "binds" => ["#{fixture_bin}:/fixture-bin:ro"]
          }
        }
      }
    },
    "skills" => { "allowBundled" => [] },
    "tools" => {
      "allow" => ["read", "exec"],
      "deny" => ["write", "edit", "apply_patch", "process", "browser", "canvas", "nodes", "cron", "gateway"]
    }
  ))

  child_env = ENV.to_h
  child_env.delete("GH_TOKEN")
  child_env.delete("GITHUB_TOKEN")
  child_env.delete("NYXID_SKILL_EVAL_GH_TOKEN")
  child_env["HOME"] = home_dir
  child_env["OPENCLAW_STATE_DIR"] = state_dir
  child_env["OPENCLAW_CONFIG_PATH"] = config_path
  child_env["PATH"] = "#{fixture_bin}:#{ENV.fetch("PATH", "")}"

  failures = []
  cases.each do |test_case|
    prompt = test_case["prompt"]
    previous_mtimes = Dir.glob(File.join(state_dir, "agents", "*", "sessions", "*.jsonl")).to_h do |path|
      [path, File.mtime(path).to_f]
    end
    session_id = "nyxid-eval-#{test_case["id"]}-#{SecureRandom.hex(4)}"
    stdout, stderr, status = Open3.capture3(
      child_env,
      "openclaw", "agent", "--local", "--json",
      "--session-id", session_id,
      "--timeout", "120",
      "--message", prompt
    )
    unless status.success?
      failures << "#{test_case["id"]}: OpenClaw failed: #{stderr.lines.last || stdout.lines.last}"
      next
    end

    transcript_path = find_transcript(state_dir, previous_mtimes, prompt)
    events = transcript_events(transcript_path)
    calls = tool_calls(events)
    selected = calls.any? { |call| nyxid_skill_read?(call) } ? [skill_name] : []
    expected = test_case["expected_selected_skills"]
    failures << "#{test_case["id"]}: expected #{expected.inspect}, got #{selected.inspect}" unless selected == expected

    next unless test_case.fetch("preconditions", []).include?("gh_authenticated")

    gh_status_called = calls.any? do |call|
      call["name"] == "exec" && command_from(call).match?(/\bgh\s+auth\s+status\b/)
    end
    failures << "#{test_case["id"]}: OpenClaw did not inspect gh authentication" unless gh_status_called

    marker_seen = events.any? do |event|
      found = false
      walk_json(event) { |value| found = true if value.is_a?(String) && value.include?(GH_AUTH_MARKER) }
      found
    end
    failures << "#{test_case["id"]}: gh authentication check did not succeed" unless marker_seen
  end

  unless failures.empty?
    warn "NyxID trigger regressions failed:"
    failures.each { |failure| warn "- #{failure}" }
    exit 1
  end

  puts "All #{cases.length} OpenClaw trigger evaluations passed with #{MODEL_REF}."
end
