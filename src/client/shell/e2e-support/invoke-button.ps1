# Invokes the named button inside ONE process's window through Windows UI Automation. It acts only
# on that process's own element tree (never global keyboard or mouse input), so it cannot reach
# another application. Exit status: 0 invoked, 3 no such button.
# Why UI Automation: Chromium shows the HTTPS-First "Continue to site" prompt as a browser-UI
# dialog, not a web page, so it is absent from the DOM, the accessibility tree DevTools exposes
# and screenshots of the page; only the operating system's accessibility API reaches it.
param(
  [Parameter(Mandatory = $true)][int]$ProcId,
  [Parameter(Mandatory = $true)][string]$ButtonName
)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::RootElement
$byProcess = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $ProcId)
$byName = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $ButtonName)
$isButton = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
$match = New-Object System.Windows.Automation.AndCondition($byName, $isButton)
foreach ($window in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $byProcess)) {
  $button = $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $match)
  if ($null -ne $button) {
    $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    Write-Output "INVOKED"
    exit 0
  }
}
Write-Output "NOT FOUND"
exit 3
