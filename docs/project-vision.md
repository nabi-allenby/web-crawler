# Web crawler vision
Create a free, open-source, deployable platform for Red & Blue teams that want to discover the web attack surface of their applications.

## About
This file should be used as general guidelines for development. When design decisions are made, this doc should define the "spirit" of those decisions.

## My philosophy
1. Don't reinvent the wheel - There is code written by smarter people than you. Be humble and use well-established code and tools.
2. Open Source - This platform should be open and transparent for everyone to contribute, share, and use.
3. Respect others - Use this platform for the betterment of software and products. Make the world better than you found it.
4. Have fun - The process of creating things should be fun. There will be chores, but enjoy the process.


## Design Principles (Derived from above)
These principles are a collection of coding and design rules I personally came across and found to work. A lot of this is based on other people's design principles.

---

### Don't reinvent the wheel

#### Adopt mainstream tools
Use well-established tools from other open-source projects. Only create custom tools when it's absolutely necessary.

#### Keep it simple stupid
Keep the project as simple as possible. The more moving parts, the less scalable it becomes, and the more things break.

### Open Source

#### All source code is public
The project vision is to be an open source platform for blue & red teams, anyone can contribute.

#### All source code should be free for individuals
This platform should always be free for individuals, and for the foreseeable future, for anyone. The code license should reflect that.

### Respect others

#### Respectful crawling
Rate limiting, robots.txt awareness, and polite user-agent strings by default. The tool should be hard to misuse for DoS or abuse.

### Have fun

#### Visualization graph should be fun to use and explore
The visuals and tools for exploring the graph should be fun for the user, possibly gamified.

#### Project theme should be fun
The theme of this project should be cartoony, playful, and fun. The main theme is cobweb (as it's a crawler).

