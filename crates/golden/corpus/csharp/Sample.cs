using System;
using System.Collections.Generic;

namespace Sample.App;

/// <summary>A tiny greeter used by the golden corpus.</summary>
public sealed class Greeter
{
    private readonly string _name;
    private readonly List<string> _history = new();

    public Greeter(string name)
    {
        _name = name;
    }

    public string Greet(int times = 1)
    {
        var text = "";
        for (var i = 0; i < times; i++)
        {
            text += $"Hello, {_name}!";
            _history.Add(text);
        }
        Console.WriteLine(text);
        return text;
    }
}
