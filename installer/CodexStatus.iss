#ifndef AppVersion
  #define AppVersion "0.6.1"
#endif

#define AppName "CodexStatus"
#define AppPublisher "CodexStatus contributors"
#define AppURL "https://github.com/mmm1h/codex-status"
#define AppExeName "CodexStatus.exe"

[Setup]
AppId={{4B7D5A91-45A5-4B78-A095-A9B43A2A4F7D}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
DefaultDirName={localappdata}\Programs\CodexStatus
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=CodexStatus-v{#AppVersion}-Setup-x64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\{#AppExeName}
CloseApplications=yes
RestartApplications=no
AppMutex=Local\CodexStatus.4B7D5A91-45A5-4B78-A095-A9B43A2A4F7D
MinVersion=10.0.17763

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startup"; Description: "Start CodexStatus with Windows"; GroupDescription: "Startup:"; Flags: checkedonce

[Files]
Source: "..\dist\CodexStatus.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "CodexStatus"; ValueData: """{app}\{#AppExeName}"" --background"; Tasks: startup; Flags: uninsdeletevalue

[Icons]
Name: "{group}\CodexStatus"; Filename: "{app}\{#AppExeName}"
Name: "{group}\Uninstall CodexStatus"; Filename: "{uninstallexe}"

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Launch CodexStatus"; Flags: nowait postinstall skipifsilent
