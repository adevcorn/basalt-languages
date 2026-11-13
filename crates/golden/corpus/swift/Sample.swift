import Foundation

struct Greeter {
    let name: String
    var count: Int = 0

    func greet(times: Int = 1) -> String {
        var text = ""
        for _ in 0..<times {
            text += "Hello, \(name)!"
        }
        print(text)
        return text
    }
}

let greeter = Greeter(name: "Basalt")
print(greeter.greet(times: 2))
