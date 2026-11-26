Pod::Spec.new do |s|
  s.name           = 'NyxWatch'
  s.version        = '1.0.0'
  s.summary        = 'Apple Watch companion link for the NyxID app'
  s.description    = 'Reports Apple Watch pairing state and hands the NyxID watch app a one-time sign-in code over WatchConnectivity.'
  s.license        = { :type => 'Proprietary' }
  s.author         = 'NyxID'
  s.homepage       = 'https://github.com/ChronoAIProject/NyxID'
  s.platforms      = {
    :ios => '15.1'
  }
  s.swift_version  = '5.9'
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'
  s.frameworks = 'WatchConnectivity'

  s.source_files = "**/*.{h,m,swift}"
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_COMPILATION_MODE' => 'wholemodule'
  }
end
