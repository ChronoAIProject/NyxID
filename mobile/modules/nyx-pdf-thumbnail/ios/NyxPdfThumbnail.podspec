Pod::Spec.new do |s|
  s.name = 'NyxPdfThumbnail'
  s.version = '1.0.0'
  s.summary = 'Local first-page PDF thumbnails for selected chat attachments'
  s.license = { :type => 'Apache-2.0' }
  s.author = 'NyxID'
  s.homepage = 'https://github.com/ChronoAIProject/NyxID'
  s.platform = :ios, '15.1'
  s.source = { :git => 'https://github.com/ChronoAIProject/NyxID.git' }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.source_files = '**/*.swift'
  s.swift_version = '5.9'
end
