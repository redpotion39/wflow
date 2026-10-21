#define MyAppName "Wflow"
#define MyAppVersion "0.3.1"
#define MyAppPublisher "Wflow Team"
#define MyAppExeName "wflow.exe"

[Setup]
AppId={{C78F5960-A6E5-47F1-9B54-68FE11E2616A}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
OutputDir=dist
OutputBaseFilename=Wflow-Setup-{#MyAppVersion}
SetupIconFile=assets\app_icon.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
PrivilegesRequiredOverridesAllowed=dialog
PrivilegesRequired=lowest
UninstallDisplayIcon={app}\{#MyAppExeName}
CloseApplications=force
CloseApplicationsFilter=*.exe
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "startupicon"; Description: "Launch Wflow automatically when Windows starts"; GroupDescription: "System Integration:"; Flags: unchecked

[Files]
Source: "target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\app_icon.ico"; DestDir: "{app}\assets"; Flags: ignoreversion
Source: "assets\app_icon.png"; DestDir: "{app}\assets"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"
Name: "{autoprograms}\{#MyAppName}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"; Tasks: desktopicon
Name: "{userstartup}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"; Tasks: startupicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Code]
// Force terminate any running wflow.exe instances so files are not locked
procedure StopWflowProcesses();
var
  ResultCode: Integer;
begin
  Exec('taskkill.exe', '/F /IM wflow.exe /T', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Sleep(400);
end;

function InitializeSetup(): Boolean;
begin
  StopWflowProcesses();
  Result := True;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
  begin
    StopWflowProcesses();
  end;
end;

function InitializeUninstall(): Boolean;
begin
  StopWflowProcesses();
  Result := True;
end;

