Pod::Spec.new do |s|
  s.name = 'NyxPerformance'
  s.version = '1.0.0'
  s.summary = 'Consent-scoped native startup and frame timing'
  s.description = 'Bounded native timing only, without content, persistence or transport.'
  s.license = { :type => 'Apache-2.0' }
  s.author = 'NyxID'
  s.homepage = 'https://github.com/ChronoAIProject/NyxID'
  s.platform = :ios, '15.1'
  s.source = { :git => 'https://github.com/ChronoAIProject/NyxID.git' }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.source_files = '**/*.{swift,h,m}'
  s.swift_version = '5.9'
end
