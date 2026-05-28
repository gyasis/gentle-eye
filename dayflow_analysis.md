**Chunk 1**
Lines added: 258, lines removed: 1
@@ -1,1 +1,258 @@
+ # Dayflow - High-Level Analysis
+ ## Screen Recording App Feature Breakdown
+ **Analysis Date:** December 21, 2025  
+ **Source:** dayflow.so, GitHub repositories, research documentation
+ ## Overview
+ **Dayflow** is a native macOS application that automatically generates a visual timeline of your daily computer activities by recording your screen and analyzing it with AI. It's designed to help users understand their productivity patterns, identify distractions, and track work habits.
+ **Key Value Proposition:** Automatic, privacy-focused screen recording with AI-powered activity analysis and timeline visualization.
+ ## Core Architecture: 5-Stage Pipeline
+ Dayflow uses a structured pipeline to transform raw screen captures into meaningful insights:
+ ### 1. **Capture Stage**
+ - **Technology:** macOS ScreenCaptureKit API
+ - **Recording Rate:** 1 frame per second (1 FPS)
+ - **Storage Format:** 15-second video chunks saved temporarily to disk
+ - **Why 1 FPS?** Minimizes CPU usage, storage requirements, and battery drain while still capturing enough visual information for AI analysis
+ - **Permissions Required:** Screen &amp; System Audio Recording permission on macOS
+ - **Known Limitation:** Multi-screen setups may only record the screen where the app is opened
+ ### 2. **Analyze Stage**
+ - **Frequency:** Every 15 minutes
+ - **Process:** Sends recent footage chunks to AI for analysis
+ - **AI Provider Options:**
+   - **Gemini** (bring your own API key)
+   - **Local models** (Ollama / LM Studio) - for privacy
+   - **ChatGPT/Claude** (requires paid subscription)
+ - **What AI Analyzes:**
+   - On-screen activities
+   - Application usage
+   - Distinguishes between productive work and distractions
+   - Contextual understanding (e.g., programming tutorial vs. unrelated video)
+ ### 3. **Summarize Stage**
+ - **Output:** Timeline cards with activity summaries
+ - **Content:** Concise descriptions of what happened during each time period
+ - **Intelligence:** AI generates semantic summaries, not just raw data
+ ### 4. **Visualize Stage**
+ - **Presentation:** Visual timeline of the day
+ - **Color Coding:** Highlights different activity types:
+   - Deep work periods
+   - Shallow tasks
+   - Breaks
+   - Distractions
+ - **User Interface:** Built with SwiftUI for native macOS experience
+ - **Timelapse Playback:** Users can watch timelapses of their day
+ ### 5. **Manage Stage**
+ - **Storage Management:** Automatic cleanup based on configurable limits
+ - **Storage Options:** 1GB
20GB or unlimited
+ - **Retention Policy:** Automatically deletes recordings after 3 days (configurable)
+ - **Efficiency:** 1 FPS recording significantly reduces storage needs compared to standard screen recordings
+ ## Key Features Breakdown
+ ### Automatic Timeline Generation
+ - **What it does:** Creates a chronological visual representation of your day
+ - **How it works:** Combines screen recordings with AI analysis to generate timeline cards
+ - **User Benefit:** See your entire day at a glance without manual logging
+ ### AI-Powered Activity Understanding
+ - **Capability:** Distinguishes between different types of activities
+ - **Examples:**
+   - Productive work vs. distractions
+   - Educational content (tutorials) vs. entertainment
+   - Deep work vs. shallow tasks
+ - **Technology:** Vision-language models analyze screen content semantically
+ ### Distraction Detection
+ - **What it identifies:** Periods where user may have been distracted
+ - **How it works:** AI analyzes screen content to detect non-work-related activities
+ - **Visualization:** Highlights distraction periods in the timeline
+ - **Use Case:** Helps users identify productivity killers (social media, unrelated websites, etc.)
+ ### Privacy-Focused Design
+ - **Local Processing:** All data processing occurs on-device when using local models
+ - **On-Device AI:** Supports local models (Ollama/LM Studio) for complete privacy
+ - **Data Retention:** Automatic cleanup ensures recordings don't accumulate
+ - **Open Source:** MIT license allows code inspection and modification
+ - **User Control:** Users choose their AI provider and can opt for local-only mode
+ ### Timeline Export
+ - **Format:** Markdown export for any date range
+ - **Use Case:** Share summaries, create reports, or archive activity data
+ ### Daily Journal (Beta)
+ - **Morning Intentions:** Set goals for the day
+ - **Evening Reflections:** Reflect on what was accomplished
+ - **AI Summaries:** Automated summaries of daily activities
+ - **Scheduled Reminders:** Notifications for journaling
+ ### Resource Efficiency
+ - **App Size:** Small footprint (~25MB)
+ - **CPU Usage:** Minimal due to 1 FPS recording
+ - **RAM Usage:** Lightweight operation
+ - **Storage:** Efficient chunk-based storage with automatic cleanup
+ - **Battery Impact:** Low impact due to optimized recording rate
+ ## Technical Stack
+ - **Language:** Swift
+ - **UI Framework:** SwiftUI
+ - **Database:** SQLite (local)
+ - **Screen Capture:** macOS ScreenCaptureKit API
+ - **AI Integration:** 
+   - Gemini API
+   - Local models (Ollama/LM Studio)
+   - ChatGPT/Claude APIs
+ - **Platform:** macOS only (native application)
+ ## Data Flow
+ &#96;&#96;&#96;
+ Screen Activity
+ ScreenCaptureKit (1 FPS recording)
+ 15-second video chunks (temporary storage)
+ AI Analysis (every 15 minutes)
+ Timeline Cards with Summaries
+ Visual Timeline Display
+ Automatic Cleanup (after 3 days or storage limit)
+ &#96;&#96;&#96;
+ ## Design Decisions &amp; Trade-offs
+ ### Why 1 FPS?
+ - **Pros:** Minimal CPU/storage/battery impact, still enough for AI understanding
+ - **Cons:** Not suitable for detailed video playback, but sufficient for activity analysis
+ ### Why 15-minute Analysis Intervals?
+ - **Pros:** Balances real-time insights with resource efficiency
+ - **Cons:** Not instant feedback, but provides regular updates
+ ### Why 15-second Chunks?
+ - **Pros:** Efficient processing, manageable file sizes
+ - **Cons:** May miss very brief activities, but captures overall patterns
+ ### Why Local AI Option?
+ - **Pros:** Complete privacy, no data leaves device
+ - **Cons:** Requires more powerful hardware, potentially less accurate than cloud models
+ ### Why 3-Day Retention?
+ - **Pros:** Balances historical data with storage efficiency
+ - **Cons:** Limited long-term history, but configurable
+ ## Use Cases
+ 1. **Productivity Tracking**
+    - Understand work habits and patterns
+    - Identify peak productivity times
+    - Track time spent on different activities
+ 2. **Distraction Management**
+    - Identify what pulls you off-task
+    - Track time spent on distracting websites/apps
+    - Understand focus patterns
+ 3. **Time Management**
+    - See how time is actually spent
+    - Make informed decisions about daily routines
+    - Optimize work schedules
+ 4. **Privacy-Conscious Monitoring**
+    - Monitor activities without cloud services
+    - Keep all data on-device
+    - Full control over data processing
+ 5. **Daily Reflection**
+    - Review day's activities
+    - Set intentions and reflect on accomplishments
+    - AI-generated summaries for quick review
+ ## Limitations &amp; Known Issues
+ 1. **Platform:** macOS only (not cross-platform)
+ 2. **Multi-Screen:** May only record the screen where app is opened
+ 3. **Analysis Delay:** 15-minute intervals mean not real-time
+ 4. **Local AI:** Requires capable hardware for local model processing
+ 5. **Storage:** Even with 1 FPS, long-term storage can accumulate
+ 6. **Privacy Trade-off:** Cloud AI models require sending data externally
+ ## Key Insights for Feature Design
+ ### What Makes Dayflow Effective:
+ 1. **Low-Impact Recording:** 1 FPS is a smart trade-off - enough for AI understanding, minimal resource usage
+ 2. **Chunked Processing:** 15-second chunks make analysis manageable and efficient
+ 3. **Batch Analysis:** 15-minute intervals balance responsiveness with efficiency
+ 4. **Privacy Options:** Offering local AI gives users control over their data
+ 5. **Automatic Cleanup:** Prevents storage bloat while maintaining recent history
+ 6. **Semantic Understanding:** AI doesn't just record - it understands context
+ 7. **Visual Timeline:** Makes complex data easy to understand at a glance
+ ### Design Patterns to Consider:
+ - **Progressive Analysis:** Don't analyze everything in real-time - batch it
+ - **Storage Efficiency:** Use low frame rates and automatic cleanup
+ - **User Choice:** Offer both cloud and local AI options
+ - **Visualization:** Make complex activity data easy to understand
+ - **Privacy First:** Design for on-device processing as default
+ - **Resource Awareness:** Optimize for minimal system impact
+ ## Questions to Consider for Your Implementation
+ 1. **Recording Rate:** What frame rate do you need? (Dayflow uses 1 FPS)
+ 2. **Analysis Frequency:** How often should analysis happen? (Dayflow: every 15 min)
+ 3. **Chunk Size:** How long should video chunks be? (Dayflow: 15 seconds)
+ 4. **Storage Strategy:** How long to keep recordings? (Dayflow: 3 days default)
+ 5. **AI Provider:** Cloud vs. local? (Dayflow: both options)
+ 6. **Privacy Model:** On-device by default? (Dayflow: yes, with local option)
+ 7. **Visualization:** Timeline? Calendar? List? (Dayflow: timeline)
+ 8. **Platform:** macOS only? Cross-platform? (Dayflow: macOS only)
+ 9. **Real-time vs. Batch:** Immediate analysis or periodic? (Dayflow: periodic)
+ 10. **Cursor Tracking:** Do you need cursor detection? (Dayflow: not explicitly mentioned)
+ ## References
+ - **Official Website:** dayflow.so
+ - **GitHub Repository:** JerryZLiu/Dayflow (macOS app)
+ - **License:** MIT (open source)
+ - **Platform:** macOS native application
+ **Note:** This analysis is based on research conducted on December 21, 2025. For the most current information, refer to the official Dayflow website and GitHub repository.
</details>
</tool-use>
Created a separate analysis document for Dayflow at `dayflow_analysis.md`. It includes: