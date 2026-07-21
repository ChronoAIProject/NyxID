# frozen_string_literal: true

require "json"
require "net/http"
require "optparse"
require "uri"
require "yaml"

options = { validate_only: false }
OptionParser.new do |parser|
  parser.on("--validate-only", "Validate the skill metadata and evaluation cases without calling a model") do
    options[:validate_only] = true
  end
end.parse!

skill_path = File.expand_path("../SKILL.md", __dir__)
cases_path = File.join(__dir__, "trigger-regression.json")

skill_source = File.read(skill_path)
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
abort("#{skill_path}: frontmatter name must be a non-empty string") unless skill_name.is_a?(String) && !skill_name.empty?
abort("#{skill_path}: frontmatter description must be a non-empty string") unless description.is_a?(String) && !description.empty?

evaluation = JSON.parse(File.read(cases_path))
abort("#{cases_path}: schema_version must be 1") unless evaluation["schema_version"] == 1
abort("#{cases_path}: skill must match #{skill_name.inspect}") unless evaluation["skill"] == skill_name

cases = evaluation["cases"]
abort("#{cases_path}: cases must be a non-empty array") unless cases.is_a?(Array) && !cases.empty?

case_ids = cases.map { |test_case| test_case["id"] if test_case.is_a?(Hash) }
abort("#{cases_path}: every case must have a non-empty string id") unless case_ids.all? { |id| id.is_a?(String) && !id.empty? }
abort("#{cases_path}: case ids must be unique") unless case_ids.uniq.length == case_ids.length

cases.each do |test_case|
  id = test_case["id"]
  prompt = test_case["prompt"]
  context = test_case["context"]
  expected = test_case["expected_selected_skills"]
  abort("#{cases_path}: #{id} prompt must be a non-empty string") unless prompt.is_a?(String) && !prompt.empty?
  abort("#{cases_path}: #{id} context must be an object") unless context.is_a?(Hash)
  next if expected == [] || expected == [skill_name]

  abort("#{cases_path}: #{id} expected_selected_skills must be [] or [#{skill_name.inspect}]")
end

puts "Validated #{cases.length} trigger cases for #{skill_name}."
exit 0 if options[:validate_only]

api_key = ENV["NYXID_SKILL_EVAL_API_KEY"]
base_url = ENV["NYXID_SKILL_EVAL_BASE_URL"]
model = ENV["NYXID_SKILL_EVAL_MODEL"]
abort("NYXID_SKILL_EVAL_API_KEY is required") if api_key.nil? || api_key.empty?
abort("NYXID_SKILL_EVAL_BASE_URL is required") if base_url.nil? || base_url.empty?
abort("NYXID_SKILL_EVAL_MODEL is required") if model.nil? || model.empty?

model_cases = cases.map do |test_case|
  {
    "id" => test_case["id"],
    "user_request" => test_case["prompt"],
    "execution_context" => test_case["context"]
  }
end

router_prompt = <<~PROMPT
  Decide independently whether to load the candidate skill for each case.
  Base each decision only on the candidate description, the user request, and the factual execution context.
  The skill is optional: return an empty selected_skills array when the description does not apply.
  Do not perform any user request.

  Candidate skill:
  #{JSON.pretty_generate({ "name" => skill_name, "description" => description })}

  Cases:
  #{JSON.pretty_generate(model_cases)}

  Return only valid JSON in this exact shape, with one result for every case:
  {"results":[{"id":"case-id","selected_skills":["#{skill_name}"]}]}
  Each selected_skills value must be either [] or ["#{skill_name}"].
PROMPT

endpoint = URI("#{base_url.sub(%r{/+\z}, "")}/chat/completions")
request = Net::HTTP::Post.new(endpoint)
request["Authorization"] = "Bearer #{api_key}"
request["Content-Type"] = "application/json"
request["User-Agent"] = "nyxid-skill-trigger-evals"
request.body = JSON.generate(
  "model" => model,
  "temperature" => 0,
  "messages" => [
    {
      "role" => "system",
      "content" => "You are a skill router. Follow the candidate skill's trigger description exactly."
    },
    { "role" => "user", "content" => router_prompt }
  ]
)

http = Net::HTTP.new(endpoint.host, endpoint.port)
http.use_ssl = endpoint.scheme == "https"
http.open_timeout = 15
http.read_timeout = 120
response = http.request(request)
unless response.is_a?(Net::HTTPSuccess)
  abort("Skill evaluation request failed with HTTP #{response.code}: #{response.body.to_s[0, 500]}")
end

response_body = JSON.parse(response.body)
model_content = response_body.dig("choices", 0, "message", "content")
abort("Skill evaluation response did not contain choices[0].message.content") unless model_content.is_a?(String)

model_content = model_content.strip
model_content = model_content.sub(/\A```(?:json)?\s*/i, "").sub(/\s*```\z/, "")
model_result = JSON.parse(model_content)
results = model_result["results"]
abort("Skill evaluation model output must contain a results array") unless results.is_a?(Array)

actual_by_id = {}
results.each do |result|
  abort("Skill evaluation result must be an object") unless result.is_a?(Hash)

  id = result["id"]
  selected = result["selected_skills"]
  abort("Skill evaluation returned an unknown or duplicate case id: #{id.inspect}") unless case_ids.include?(id) && !actual_by_id.key?(id)
  abort("Skill evaluation returned invalid selected_skills for #{id}") unless selected == [] || selected == [skill_name]

  actual_by_id[id] = selected
end

missing_ids = case_ids - actual_by_id.keys
abort("Skill evaluation omitted cases: #{missing_ids.join(", ")}") unless missing_ids.empty?

failures = cases.each_with_object([]) do |test_case, collected|
  expected = test_case["expected_selected_skills"]
  actual = actual_by_id[test_case["id"]]
  next if actual == expected

  collected << "#{test_case["id"]}: expected #{expected.inspect}, got #{actual.inspect}"
end

unless failures.empty?
  warn "NyxID trigger regressions failed:"
  failures.each { |failure| warn "- #{failure}" }
  exit 1
end

puts "All #{cases.length} model trigger evaluations passed with #{model}."
