#!/usr/bin/env ruby
# Check the shipped bridge and its trust bindings without starting containers.
# Only the nested ${NAME:-default} expressions used by these fields are expanded;
# this is a policy test, not a replacement for `docker compose config`.
require 'ipaddr'
require 'minitest/autorun'
require 'yaml'

class ComposeIngressPolicy < Minitest::Test
  COMPOSE = YAML.safe_load(File.read(File.expand_path('../../docker-compose.prod.yml', __dir__)))
  NETWORK = COMPOSE.fetch('networks').fetch('frontend_backend').fetch('ipam').fetch('config').fetch(0)
  BACKEND = COMPOSE.fetch('services').fetch('backend')
  FRONTEND = COMPOSE.fetch('services').fetch('frontend')

  def expand(value, env)
    while value.include?('${')
      expanded = value.gsub(/\$\{([A-Z][A-Z0-9_]*):-([^{}]*)\}/) do
        supplied = env[Regexp.last_match(1)]
        supplied.nil? || supplied.empty? ? Regexp.last_match(2) : supplied
      end
      raise 'unsupported Compose expression' if expanded == value
      value = expanded
    end
    value
  end

  def profile(env = {})
    NETWORK.transform_keys(&:to_sym).transform_values { |v| expand(v, env) }.merge(
      frontend: expand(FRONTEND.fetch('networks').fetch('frontend_backend').fetch('ipv4_address'), env),
      backend_trust: expand(BACKEND.fetch('environment').fetch('TRUSTED_PROXY_IPS'), env),
      outer_trust: expand(FRONTEND.fetch('environment').fetch('NYXID_TRUSTED_INGRESS_IPS'), env)
    )
  end

  def assert_valid_bridge(p)
    subnet, gateway, frontend, pool = p.values_at(:subnet, :gateway, :frontend, :ip_range).map { |v| IPAddr.new(v) }
    assert subnet.ipv4? && gateway.ipv4? && frontend.ipv4? && pool.ipv4?
    assert subnet.include?(gateway), 'gateway must be inside subnet'
    assert subnet.include?(frontend), 'frontend must be inside subnet'
    assert subnet.include?(pool.to_range.first) && subnet.include?(pool.to_range.last), 'dynamic pool must be inside subnet'
    refute_equal gateway, frontend
    refute pool.include?(gateway), 'dynamic pool must exclude gateway'
    refute pool.include?(frontend), 'dynamic pool must exclude frontend'
  end

  def relocated
    { 'NYXID_FRONTEND_SUBNET' => '172.31.42.0/24', 'NYXID_FRONTEND_GATEWAY' => '172.31.42.1',
      'NYXID_FRONTEND_IP' => '172.31.42.10', 'NYXID_FRONTEND_IP_RANGE' => '172.31.42.128/25' }
  end

  def test_default_bridge_trusts_only_fixed_frontend_and_gateway
    p = profile
    assert_valid_bridge(p)
    assert_equal '172.30.0.0/24', p[:subnet]
    assert_equal '172.30.0.128/25', p[:ip_range]
    assert_equal '172.30.0.10', p[:backend_trust]
    assert_equal '172.30.0.1', p[:outer_trust]
    assert_equal p[:frontend], p[:backend_trust]
    assert_equal p[:gateway], p[:outer_trust]
  end

  def test_relocated_bridge_derives_both_trust_defaults
    p = profile(relocated)
    assert_valid_bridge(p)
    assert_equal '172.31.42.10', p[:backend_trust]
    assert_equal '172.31.42.1', p[:outer_trust]
    assert_equal '172.31.42.0/24', p[:subnet]
    assert_equal '172.31.42.128/25', p[:ip_range]
  end

  def test_explicit_trust_overrides_take_precedence
    p = profile(relocated.merge('TRUSTED_PROXY_IPS' => '192.0.2.10', 'NYXID_TRUSTED_INGRESS_IPS' => '192.0.2.1'))
    assert_valid_bridge(p)
    assert_equal '192.0.2.10', p[:backend_trust]
    assert_equal '192.0.2.1', p[:outer_trust]
    assert_equal '172.31.42.10', p[:frontend]
    assert_equal '172.31.42.1', p[:gateway]
  end

  def test_empty_trust_overrides_follow_compose_colon_default_semantics
    p = profile(relocated.merge('TRUSTED_PROXY_IPS' => '', 'NYXID_TRUSTED_INGRESS_IPS' => ''))
    assert_equal p[:frontend], p[:backend_trust]
    assert_equal p[:gateway], p[:outer_trust]
    defaults = profile(relocated.transform_values { '' })
    assert_equal profile, defaults
  end

  def test_trust_expressions_reference_the_actual_address_variables
    assert_equal '${NYXID_FRONTEND_SUBNET:-172.30.0.0/24}', NETWORK.fetch('subnet')
    assert_equal '${NYXID_FRONTEND_IP_RANGE:-172.30.0.128/25}', NETWORK.fetch('ip_range')
    assert_equal '${NYXID_FRONTEND_IP:-172.30.0.10}', FRONTEND.fetch('networks').fetch('frontend_backend').fetch('ipv4_address')
    assert_equal '${TRUSTED_PROXY_IPS:-${NYXID_FRONTEND_IP:-172.30.0.10}}', BACKEND.fetch('environment').fetch('TRUSTED_PROXY_IPS')
    assert_equal '${NYXID_TRUSTED_INGRESS_IPS:-${NYXID_FRONTEND_GATEWAY:-172.30.0.1}}', FRONTEND.fetch('environment').fetch('NYXID_TRUSTED_INGRESS_IPS')
    assert_equal '${NYXID_FRONTEND_GATEWAY:-172.30.0.1}', NETWORK.fetch('gateway')
  end

  def test_loopback_publish_and_separate_mongodb_network_are_preserved
    assert_equal ['127.0.0.1:3000:8080'], FRONTEND.fetch('ports')
    assert_equal ['frontend_backend'], FRONTEND.fetch('networks').keys
    assert_equal %w[default frontend_backend], BACKEND.fetch('networks').keys.sort
    base = YAML.safe_load(File.read(File.expand_path('../../docker-compose.yml', __dir__)))
    assert_nil base.fetch('services').fetch('mongodb')['networks']
    assert_nil COMPOSE.fetch('services').fetch('mongodb')['networks']
  end

  def test_misaligned_network_fixtures_fail_policy_validation
    [relocated.merge('NYXID_FRONTEND_IP' => '192.0.2.10'),
     relocated.merge('NYXID_FRONTEND_GATEWAY' => '192.0.2.1'),
     relocated.merge('NYXID_FRONTEND_IP_RANGE' => '172.31.42.0/24'),
     relocated.merge('NYXID_FRONTEND_IP_RANGE' => '172.31.43.128/25')].each do |env|
      # Use a separate assertion context so intentional negative assertions do
      # not inflate this test's own failure count.
      probe = self.class.new('negative_bridge_fixture')
      assert_raises(Minitest::Assertion) { probe.assert_valid_bridge(profile(env)) }
    end
  end
end
