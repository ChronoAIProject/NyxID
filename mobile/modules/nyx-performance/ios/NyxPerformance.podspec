Pod::Spec.new do |s|
  s.name = 'NyxPerformance'
  s.version = '1.0.0'
  s.summary = 'Consent-scoped native launch and display cadence'
  s.description = 'Bounded native launch milestones and display cadence measurements, with a destination-bound consent mirror and no content or transport.'
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
