export async function fetchFactorioVersions(): Promise<string[]> {
    const result = await fetch('https://raw.githubusercontent.com/wube/factorio-data/master/changelog.txt')
    const text = await result.text()
    const marker = "Version: "
    return text.split("\n").filter(line => line.startsWith(marker)).map(line => line.substr(marker.length))
}