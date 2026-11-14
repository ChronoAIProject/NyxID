Pod::Spec.new do |s|
  s.name = 'NyxCrashReporter'
  s.version = '1.0.0'
  s.summary = 'Consent-bound first-party crash capture'
  s.description = 'Closed crash records, without vendor transport or exception text.'
  s.license = { :type => 'Apache-2.0' }
  s.author = 'NyxID'
  s.homepage = 'https://github.com/ChronoAIProject/NyxID'
  s.platform = :ios, '15.1'
  s.source = { :git => 'https://github.com/ChronoAIProject/NyxID.git' }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.dependency 'PLCrashReporter', '1.12.0'
  s.source_files = '**/*.{swift,h,m}'
  s.swift_version = '5.9'
end
