using System.Text.RegularExpressions;
using StoreLib.Models;
using StoreLib.Services;

if (args.Length != 2 || !Regex.IsMatch(args[0], @"\A[A-Z0-9]{12}\z"))
    throw new ArgumentException("Pass the public Microsoft Store product ID and output package path.");

const string packageName = "pan2013e.Meta-Harness";
var catalog = new DisplayCatalogHandler(DCatEndpoint.Production, new Locale(Market.US, Lang.en, false));
await catalog.QueryDCATAsync(args[0]);
if (!catalog.IsFound)
    throw new InvalidOperationException("The Microsoft Store product is not public.");

var knownPackages = await catalog.GetPackagesForProductAsync();
var package = knownPackages
    .Select(item => (Item: item, Parts: item.PackageMoniker.Split('_')))
    .Where(entry => entry.Parts.Length >= 3
        && entry.Parts[0].Equals(packageName, StringComparison.OrdinalIgnoreCase)
        && (entry.Parts[2] == "x64" || entry.Parts is [_, _, "neutral", "~", ..]))
    .Select(entry => (entry.Item.PackageUri, Version: Version.Parse(entry.Parts[1]), IsX64: entry.Parts[2] == "x64"))
    .OrderByDescending(entry => entry.Version)
    .ThenByDescending(entry => entry.IsX64)
    .FirstOrDefault();
if (package.PackageUri is null)
    throw new InvalidOperationException("The Store did not offer a Meta-Harness package for x64.");

var uri = package.PackageUri;
// Some Store CDN packages are served over HTTP. The release job verifies the
// downloaded package's signature and checks it against the catalog version.
if (uri.Scheme is not ("http" or "https") || !uri.IsDefaultPort || uri.UserInfo.Length != 0
    || !uri.Host.EndsWith(".delivery.mp.microsoft.com", StringComparison.OrdinalIgnoreCase))
    throw new InvalidOperationException("The Store package URL is outside Microsoft's delivery service.");

using var http = new HttpClient(new HttpClientHandler { AllowAutoRedirect = false });
using var timeout = new CancellationTokenSource(TimeSpan.FromMinutes(8));
using var response = await http.GetAsync(uri, HttpCompletionOption.ResponseHeadersRead, timeout.Token);
response.EnsureSuccessStatusCode();
await using var output = new FileStream(args[1], FileMode.CreateNew);
await response.Content.CopyToAsync(output, timeout.Token);
await File.WriteAllTextAsync(Path.ChangeExtension(args[1], ".version"), package.Version.ToString(), timeout.Token);
Console.WriteLine($"Downloaded Store package {package.Version}.");
